#!/bin/sh
# Starts a local mindwiki-server for testing. No certificate or domain is
# needed: the server makes its own (self-signed) HTTPS certificate.
BIN="$(dirname "$0")/target/release/mindwiki-server"
DATA="$HOME/mindwiki-server-data"

while [ "$("$BIN" user list --data "$DATA")" = "no accounts" ]; do
    echo "Create your account."
    echo "The password needs 8 or more characters. It stays hidden while you type."
    printf "Username: "
    read -r NAME
    "$BIN" user add "$NAME" --data "$DATA" || { echo; echo "That didn't work, let's try again."; echo; }
done
echo

echo "In mindwiki click the cloud button and enter:"
echo "  Server:      https://localhost:8443"
echo "  Fingerprint: the long line printed below"
echo "Press Ctrl+C here to stop the server."
echo
"$BIN" serve --data "$DATA"
echo
echo "Server stopped. Press Enter to close."
read -r _
