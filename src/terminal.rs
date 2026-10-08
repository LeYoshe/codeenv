//! WebSocket ↔ ptyd bridge for one browser view of a terminal.
//!
//! Protocol: binary frames carry raw terminal bytes in both directions,
//! untouched. Text frames carry JSON control messages:
//!   client → server  {"type":"resize","cols":N,"rows":N}
//!   server → client  {"type":"exit"}   the program has exited

use std::sync::Arc;

use anyhow::{Result, bail};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

use crate::App;
use crate::ptyd::{self, KIND_DATA, KIND_JSON, Request, Response};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Control {
    Resize { cols: u16, rows: u16 },
}

enum End {
    /// The browser went away; the terminal keeps running.
    ClientClosed,
    /// The program exited.
    Exited,
}

pub async fn serve(app: Arc<App>, id: String, mut ws: WebSocket, cols: u16, rows: u16) {
    match bridge(&app, &id, &mut ws, cols, rows).await {
        Ok(End::ClientClosed) => {
            let _ = ws.close().await;
        }
        Ok(End::Exited) => {
            let _ = ws.send(Message::Text(r#"{"type":"exit"}"#.into())).await;
            let _ = ws.close().await;
        }
        // ptyd unreachable or protocol error: the browser retries.
        Err(e) => {
            tracing::warn!("terminal {id}: {e:#}");
            let _ = ws.close().await;
        }
    }
}

async fn bridge(app: &App, id: &str, ws: &mut WebSocket, cols: u16, rows: u16) -> Result<End> {
    let stream = app.pty.attach(id, cols, rows).await?;
    let (mut daemon_reader, mut daemon_writer) = stream.into_split();
    let (mut browser_writer, mut browser_reader) = ws.split();
    let to_browser = async {
        loop {
            match ptyd::read_frame(&mut daemon_reader).await? {
                Some((KIND_DATA, data)) => {
                    browser_writer.send(Message::Binary(data.into())).await?
                }
                Some((KIND_JSON, payload)) => {
                    let response: Response = serde_json::from_slice(&payload)?;
                    if response.event.as_deref() == Some("exit") {
                        return Ok(End::Exited);
                    }
                }
                Some((kind, _)) => bail!("unknown frame kind {kind} from ptyd"),
                // ptyd dropped us (we fell too far behind); the browser
                // reconnects and gets a fresh snapshot.
                None => bail!("ptyd closed the stream"),
            }
        }
    };
    let to_terminal = async {
        loop {
            match browser_reader.next().await {
                Some(Ok(Message::Binary(data))) => {
                    ptyd::write_frame(&mut daemon_writer, KIND_DATA, &data).await?
                }
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Control>(&text) {
                    Ok(Control::Resize { cols, rows }) => {
                        ptyd::write_json(&mut daemon_writer, &Request::Resize { cols, rows })
                            .await?
                    }
                    Err(e) => tracing::warn!("terminal {id}: bad control message: {e}"),
                },
                Some(Ok(Message::Close(_))) | None => return Ok(End::ClientClosed),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
            }
        }
    };
    // Keep each read alive until its frame is complete. Selecting individual
    // reads would discard partially received frames whenever input arrives.
    tokio::select! {
        result = to_browser => result,
        result = to_terminal => result,
    }
}
