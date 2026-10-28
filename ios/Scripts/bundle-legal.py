#!/usr/bin/env python3
"""Bundle the repository's static policy drafts; --check detects stale native text."""
import argparse
import hashlib
from datetime import datetime
from html.parser import HTMLParser
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
SCREENS = ROOT / "web/src/screens"
DESTINATION = ROOT / "ios/PinVoyager/Resources/legal.json"


class PlainText(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []
        self.links = []

    def handle_starttag(self, tag, attributes):
        if tag in ("h1", "h2", "p", "li", "aside"):
            self.parts.append("\n\n")
        if tag == "a":
            self.links.append(dict(attributes).get("href"))

    def handle_endtag(self, tag):
        if tag in ("h1", "h2", "p", "li", "aside"):
            self.parts.append("\n\n")
        if tag == "a":
            link = self.links.pop()
            if link:
                self.parts.append(f" ({link})")

    def handle_data(self, text):
        self.parts.append(text)

    def text(self):
        return "\n\n".join(
            re.sub(r"\s+", " ", block).strip()
            for block in "".join(self.parts).split("\n\n")
            if block.strip()
        )


def render():
    layout = (SCREENS / "LegalPage.tsx").read_text()
    article = re.search(r"(<article\b.*?</article>)", layout, re.S).group(1)
    updated = re.search(r"Updated ([A-Za-z]+ \d{1,2}, \d{4})", layout).group(1)
    revision = datetime.strptime(updated, "%B %d, %Y").date().isoformat()
    documents = []
    sources = [SCREENS / name for name in ("LegalPage.tsx", "Privacy.tsx", "Terms.tsx")]
    for identifier, name in (("privacy", "Privacy"), ("terms", "Terms")):
        source = (SCREENS / f"{name}.tsx").read_text()
        policy = re.search(r'<LegalPage title="([^"]+)">(.*?)</LegalPage>', source, re.S)
        markup = article.replace("{title}", policy.group(1)).replace("{children}", policy.group(2))
        markup = markup.replace('{" "}', " ")
        if "{" in markup or "}" in markup:
            raise ValueError("Policy contains JSX expressions that need explicit rendering")
        parser = PlainText()
        parser.feed(markup)
        documents.append({"id": identifier, "title": policy.group(1), "revision": revision, "text": parser.text()})
    return json.dumps({"sources": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sources}, "documents": documents}, indent=2, ensure_ascii=False) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = render()
    if args.check:
        if not DESTINATION.exists() or DESTINATION.read_text() != text:
            parser.exit(1, "Bundled legal text is stale. Run python3 ios/Scripts/bundle-legal.py.\n")
    else:
        DESTINATION.parent.mkdir(parents=True, exist_ok=True)
        DESTINATION.write_text(text)


if __name__ == "__main__":
    main()
