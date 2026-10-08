# Documentation

Start with the [project README](../README.md) for features and shortcuts.

- [Deploy on a VPS](deployment.md): build, configuration, systemd service,
  and Cloudflare access.
- [Troubleshooting](troubleshooting.md): where to find errors and what to
  check for each symptom.
- [HTTP API](api.md): requests, responses, and limits for scripts or frontend
  changes.
- [Releases and site maintenance](releasing.md): GitHub setup, CI binaries, Pages,
  and release publishing.
- [Terminal daemon protocol](protocol.md): communication between codeenv and
  `ptyd`, for work on the terminal code.
- [Tests and coverage](testing.md): tested behavior, local commands,
  and remaining gaps.

Use [config.example.toml](../config.example.toml) and [deploy/](../deploy/)
as templates. Replace user names, domains, and identifiers before installing
them.
