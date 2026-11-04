#!/bin/bash
# The Mac app's signing identity: run once, on a Mac, by whoever builds Chat Computer (not on the build's machines).
#     bash apps/desktop/make-signing-identity.sh
# It makes a self-signed code-signing identity, "Chat Computer" (it names no person, and needs no Apple account),
# keeps it in your login keychain, and gives it to GitHub as the repository's secrets MAC_SIGNING_P12 (the identity,
# in base64) and MAC_SIGNING_PASSWORD (its password), which .github/workflows/apps.yml signs the Mac app with. Signed
# with the same identity each time, the app stays the same app to macOS across updates, so what it was allowed (the
# microphone) stays allowed; signed ad hoc (as without these secrets), each build is a new app to macOS, asked again.
# With GitHub's gh signed in, the secrets are set straight away (nothing shown); otherwise the two values are left
# in a folder only you can read, to paste into the repository's Settings, Secrets and variables, Actions, and delete.
# Back the identity up (Keychain Access, My Certificates, "Chat Computer", Export): a new one makes macOS ask again.
# Nothing it makes belongs in the repository.
set -euo pipefail
name="${1:-Chat Computer}"
[ "$(uname -s)" = Darwin ] || { echo 'Run this on a Mac.' >&2; exit 1; }
if security find-identity -p codesigning | grep -qF "\"$name\""; then
  echo "\"$name\" is already in your keychain: export it from Keychain Access to set the secrets again." >&2
  exit 1
fi
work="$(mktemp -d "${TMPDIR:-/tmp}/chat-computer-signing.XXXXXX")"
chmod 700 "$work"
trap 'rm -rf "$work"' EXIT
cat > "$work/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF
openssl req -x509 -newkey rsa:3072 -sha256 -days 3650 -nodes -config "$work/cert.cnf" \
  -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
password="$(openssl rand -hex 24)"
# (macOS takes only the older PKCS#12 encryption, not OpenSSL 3's.)
openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -name "$name" -out "$work/identity.p12" \
  -passout "pass:$password" -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
security import "$work/identity.p12" -k "$HOME/Library/Keychains/login.keychain-db" -P "$password" -T /usr/bin/codesign >/dev/null
security find-identity -p codesigning | grep -qF "\"$name\"" || { echo 'The identity could not be added to the keychain.' >&2; exit 1; }
echo "Made \"$name\", in your login keychain (self-signed: macOS lists it as not trusted; codesign uses it all the same)."
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  base64 < "$work/identity.p12" | tr -d '\n' | gh secret set MAC_SIGNING_P12
  printf '%s' "$password" | gh secret set MAC_SIGNING_PASSWORD
  echo "Set the repository's secrets MAC_SIGNING_P12 and MAC_SIGNING_PASSWORD: the next Mac build is signed with it."
else
  keep="$HOME/Chat Computer signing (delete after use)"
  mkdir -p "$keep" && chmod 700 "$keep"
  base64 < "$work/identity.p12" | tr -d '\n' > "$keep/MAC_SIGNING_P12.txt"
  printf '%s' "$password" > "$keep/MAC_SIGNING_PASSWORD.txt"
  chmod 600 "$keep"/*.txt
  echo "GitHub's gh isn't signed in here: the two secrets are in \"$keep\". Paste each file's contents into the"
  echo "repository's Settings, Secrets and variables, Actions (as MAC_SIGNING_P12 and MAC_SIGNING_PASSWORD), then delete it."
fi
