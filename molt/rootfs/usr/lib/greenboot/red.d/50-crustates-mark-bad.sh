#!/bin/bash
# greenboot runs this when the health check fails. On the first boot of a new
# image, the agent marks the image bad so it is never staged again after
# greenboot rolls it back.
set -euo pipefail
if grub2-editenv list | grep -q '^greenboot_next_deployment_id='; then
    /usr/bin/crust-agent molt-mark-bad
fi
