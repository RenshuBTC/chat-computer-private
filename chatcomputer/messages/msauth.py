"""Microsoft's sign-in for Outlook.com and Hotmail mail (which no longer takes passwords over IMAP).

The person signs in on Microsoft's own page, shown between the sidebars; when Microsoft sends the browser back to
REDIRECT with a code, the window catches that address (it never loads) and the code is traded for tokens:
OAuth 2.0 authorization code with PKCE, as a public client (no secret). It needs an application id, from
registering an app in Microsoft Entra once ("Personal Microsoft accounts", a "Mobile and desktop" redirect URI of
REDIRECT, public client flows allowed, and the IMAP.AccessAsUser.All permission): settings' outlook_client_id.

Microsoft's documentation: OAuth for IMAP is available "for both Microsoft 365 ... and Outlook.com users"
(learn.microsoft.com, "Authenticate an IMAP, POP or SMTP connection using OAuth").
"""
from __future__ import annotations

import base64
import hashlib
import json
import secrets
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

TENANT = "consumers"             # personal Microsoft accounts: Outlook.com, Hotmail, Live
AUTHORIZE = f"https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/authorize"
TOKEN = f"https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/token"
REDIRECT = "https://login.microsoftonline.com/common/oauth2/nativeclient"
SCOPES = "https://outlook.office.com/IMAP.AccessAsUser.All offline_access openid email"


class SignInError(Exception):
    pass


def start(client_id: str) -> dict[str, str]:
    """Where to send the browser, and what to check the answer against."""
    verifier = secrets.token_urlsafe(48)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    state = secrets.token_urlsafe(16)
    query = urllib.parse.urlencode({
        "client_id": client_id, "response_type": "code", "redirect_uri": REDIRECT, "response_mode": "query",
        "scope": SCOPES, "state": state, "code_challenge": challenge, "code_challenge_method": "S256",
        "prompt": "select_account"})
    return {"url": f"{AUTHORIZE}?{query}", "verifier": verifier, "state": state}


def code_from(redirect_url: str, state: str) -> str:
    """The code in the address Microsoft sent the browser back to (or why there isn't one)."""
    q = urllib.parse.parse_qs(urllib.parse.urlparse(redirect_url).query)
    if q.get("error"):
        raise SignInError((q.get("error_description") or q["error"])[0][:300])
    if q.get("state", [""])[0] != state:
        raise SignInError("the sign-in answer didn't match the request")
    code = q.get("code", [""])[0]
    if not code:
        raise SignInError("Microsoft sent no sign-in code")
    return code


def _post(form: dict[str, str], opener: Any = None) -> dict[str, Any]:
    data = urllib.parse.urlencode(form).encode()
    req = urllib.request.Request(TOKEN, data=data, headers={"Content-Type": "application/x-www-form-urlencoded"})
    try:
        with (opener or urllib.request.urlopen)(req, timeout=30) as r:
            out = json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        try:
            out = json.loads(e.read().decode())
        except Exception:
            raise SignInError(f"Microsoft's sign-in answered {e.code}") from None
    except OSError as e:
        raise SignInError(f"couldn't reach Microsoft's sign-in: {e}") from None
    if out.get("error"):
        raise SignInError(str(out.get("error_description") or out["error"])[:300])
    if not out.get("access_token"):
        raise SignInError("Microsoft's sign-in gave no access token")
    out["expires_at"] = time.time() + float(out.get("expires_in") or 3600) - 60
    return out


def finish(client_id: str, code: str, verifier: str, opener: Any = None) -> dict[str, Any]:
    return _post({"client_id": client_id, "grant_type": "authorization_code", "code": code,
                  "redirect_uri": REDIRECT, "code_verifier": verifier, "scope": SCOPES}, opener)


def refresh(client_id: str, refresh_token: str, opener: Any = None) -> dict[str, Any]:
    return _post({"client_id": client_id, "grant_type": "refresh_token", "refresh_token": refresh_token,
                  "scope": SCOPES}, opener)


def address_of(tokens: dict[str, Any]) -> str:
    """The mailbox's address, from the ID token Microsoft gave with the tokens (read, not verified: it came
    straight from Microsoft over HTTPS, and only says which mailbox to open)."""
    try:
        payload = tokens["id_token"].split(".")[1]
        claims = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
    except Exception:
        return ""
    return str(claims.get("email") or claims.get("preferred_username") or "")


def xoauth2(user: str, token: str) -> bytes:
    """The IMAP AUTHENTICATE XOAUTH2 string (imaplib base64-encodes it)."""
    return f"user={user}\x01auth=Bearer {token}\x01\x01".encode()
