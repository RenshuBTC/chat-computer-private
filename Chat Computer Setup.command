#!/bin/bash
# Chat Computer Setup for Mac. Double-click this file. If macOS won't open it, open Terminal and type
#     bash ~/Downloads/ChatComputer/mac/setup.sh
# (if the Chat Computer folder isn't in Downloads, type "bash " and drag mac/setup.sh into Terminal).
cd "$(dirname "$0")" || exit 1
exec bash mac/setup.sh
