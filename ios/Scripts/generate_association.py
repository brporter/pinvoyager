#!/usr/bin/env python3
"""Generate an association document for the signed PinVoyager application identifier."""
import argparse
import json
from pathlib import Path
import re


def association(application_identifier):
    if not re.fullmatch(r"[A-Z0-9]{10}\.com\.pinvoyager\.app", application_identifier):
        raise ValueError("Use the signed application-identifier entitlement: PREFIX.com.pinvoyager.app.")
    return {"applinks": {"details": [{
        "appIDs": [application_identifier],
        "components": [{"/": path} for path in ("/voyages/*", "/u/*", "/upload", "/settings")],
    }]}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--application-identifier", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = association(args.application_identifier)
    except ValueError as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, indent=2) + "\n")


if __name__ == "__main__":
    main()
