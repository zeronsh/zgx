#!/bin/sh
# Installs Ubuntu packages on a CI runner: apt-install.sh PACKAGE...
#
# Restores the .debs cached from earlier runs (~/apt-archives), then tries the
# install three times. A stalled mirror fails fast and retries rather than
# holding the job. The runners' Azure mirror (azure.archive.ubuntu.com)
# sometimes stops resolving for hours, and every try against it then fails the
# same way until the step's timeout: so the sources move to Ubuntu's own
# archive when that name doesn't resolve, and on every try after the first.
# The .debs installed are saved back for the next run's cache.
set -u
own_archive() {
  for sources in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    if [ -f "$sources" ]; then sudo sed -i 's#azure\.archive\.ubuntu\.com#archive.ubuntu.com#g' "$sources"; fi
  done
}
sudo rm -f /var/lib/man-db/auto-update
if [ -d ~/apt-archives ]; then sudo cp ~/apt-archives/*.deb /var/cache/apt/archives/; fi
if ! getent hosts azure.archive.ubuntu.com > /dev/null; then
  echo "::warning::azure.archive.ubuntu.com doesn't resolve; installing from archive.ubuntu.com"
  own_archive
fi
apt="-o Acquire::Retries=3 -o Acquire::http::Timeout=30"
for attempt in 1 2 3; do
  if [ "$attempt" -gt 1 ]; then own_archive; fi
  # shellcheck disable=SC2086 # $apt is several options
  if sudo apt-get update -qq $apt &&
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends $apt "$@"; then
    mkdir -p ~/apt-archives && cp /var/cache/apt/archives/*.deb ~/apt-archives/
    exit 0
  fi
  if [ "$attempt" = 3 ]; then
    echo "::error::apt-get couldn't install the packages in three tries"
    exit 1
  fi
  echo "::warning::apt-get failed (try $attempt of 3); trying again from archive.ubuntu.com"
  sleep 20
done
