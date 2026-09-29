#!/usr/bin/env bash
# Debian 13 development containers without root: extract Chromium shared libraries
# into an explicit user-owned directory; never install system packages.
set -euo pipefail
if [[ $# != 1 || $1 != /* ]]; then
  echo 'Usage: tools/playwright-user-deps.sh /absolute/scratch/directory' >&2
  exit 2
fi
source /etc/os-release
if [[ ${ID:-} != debian || ${VERSION_ID:-} != 13 ]]; then
  echo 'This helper supports Debian 13 only; use Playwright install-deps on other hosts.' >&2
  exit 2
fi
root=$1
mkdir -p "$root/lists/partial" "$root/cache/archives/partial" "$root/packages" "$root/root"
cat > "$root/apt.conf" <<'CONF'
#clear APT::Update::Post-Invoke;
#clear APT::Update::Post-Invoke-Success;
CONF
apt_args=(-c "$root/apt.conf" -o "Dir::State::lists=$root/lists" -o "Dir::Cache=$root/cache")
apt-get "${apt_args[@]}" update >&2
(
  cd "$root/packages"
  apt-get "${apt_args[@]}" download libatk1.0-0t64 libatk-bridge2.0-0t64 \
    libdbus-1-3 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 \
    libxkbcommon0 libasound2t64 libatspi2.0-0t64 libdrm2 libdrm-common libxi6 >&2
)
for package in "$root"/packages/*.deb; do dpkg-deb -x "$package" "$root/root"; done
printf 'Run browser tests with LD_LIBRARY_PATH=%q${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\n' \
  "$root/root/usr/lib/$(dpkg-architecture -qDEB_HOST_MULTIARCH)"
