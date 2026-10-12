"""Verify public TLS, SPA routing, API routing, and canonical redirects."""
import json
import sys
import urllib.request
import urllib.error
import re

origin = sys.argv[1] if len(sys.argv) > 1 else "https://pinvoyager.com"

def get(path):
    with urllib.request.urlopen(origin + path, timeout=30) as response:
        assert response.status == 200, (path, response.status)
        return response.read(), response.headers

root, headers = get("/")
assert b'<div id="root">' in root and b'/assets/' in root
assert "text/html" in headers["Content-Type"]
asset = re.search(rb'src="(/assets/[^" ]+)"', root).group(1).decode()
_, asset_headers = get(asset)
assert "immutable" in asset_headers.get("Cache-Control", "")
for path in ("/assets/deployment-check-missing.js", "/auth/google/callback/deployment-check"):
    try:
        get(path)
    except urllib.error.HTTPError as error:
        assert error.code == 404, (path, error.code)
    else:
        raise AssertionError((path, "expected 404"))
try:
    request = urllib.request.Request(origin + "/auth/dev/login", data=b'{}', headers={"Content-Type": "application/json"})
    urllib.request.urlopen(request, timeout=30)
except urllib.error.HTTPError as error:
    assert error.code == 404, error.code
else:
    raise AssertionError("development login is available")
for path in ("/u/deployment-check", "/auth/callback"):
    spa, _ = get(path)
    assert spa == root, (path, "SPA deep link did not return the application")
for path in ("/health", "/ready"):
    get(path)
body, headers = get("/api/voyages")
assert "application/json" in headers["Content-Type"]
json.loads(body)
with urllib.request.urlopen(origin.replace("://", "://www.") + "/u/deployment-check", timeout=30) as response:
    assert response.url == origin + "/u/deployment-check"
print("PASS: TLS, frontend, SPA deep links, API, readiness, and www redirect")
