#!/usr/bin/env python3
"""Check local documentation links, site assets, and configuration examples."""

from html.parser import HTMLParser
import json
from pathlib import Path
import re
import tomllib
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent
errors = []


def check_link(source, link):
    url = urlsplit(link)
    if url.scheme or url.netloc:
        return
    destination = source.parent / unquote(url.path) if url.path else source
    if not destination.exists():
        errors.append(f"{source.relative_to(ROOT)}: missing {link}")
        return
    if url.fragment and destination.suffix == ".md":
        headings = re.findall(r"^#{1,6}\s+(.+)$", destination.read_text(), re.M)
        anchors = {re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-") for heading in headings}
        if unquote(url.fragment) not in anchors:
            errors.append(f"{source.relative_to(ROOT)}: missing heading {link}")


class SiteParser(HTMLParser):
    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        for name in ("href", "src", "poster"):
            if name in attributes:
                check_link(ROOT / "site/index.html", attributes[name])
        if tag == "html" and attributes.get("lang") != "en":
            errors.append("site/index.html: expected lang=en")
        if tag == "img" and not attributes.get("alt"):
            errors.append("site/index.html: image needs alternative text")


for pattern in ("*.md", "docs/*.md", "site/*.md", "tools/*/*.md"):
    for source in ROOT.glob(pattern):
        text = source.read_text()
        for link in re.findall(r"\[[^\]]*\]\(([^)]+)\)", text):
            check_link(source, link)
        for kind, body in re.findall(r"```(json|toml)\n(.*?)```", text, re.S):
            if kind == "toml":
                tomllib.loads(body)
            else:
                try:
                    json.loads(body)
                except json.JSONDecodeError:
                    for line in body.splitlines():
                        json.loads(line)
for filename in ("Cargo.toml", "rust-toolchain.toml", "config.example.toml"):
    tomllib.loads((ROOT / filename).read_text())
SiteParser().feed((ROOT / "site/index.html").read_text())
if errors:
    raise SystemExit("\n".join(errors))
print("Project links, site assets, and JSON/TOML examples are valid")
