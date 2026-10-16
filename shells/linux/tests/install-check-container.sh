#!/usr/bin/env bash
# Invoked only inside the disposable check-install.sh container.
set -Eeuo pipefail
trap 'printf "FAIL: line %s: %s\n" "$LINENO" "$BASH_COMMAND" >&2' ERR
FORMAT="$1"
ARCH="$2"
PREVIOUS="${3:-}"
case "$FORMAT" in
  deb)
    export DEBIAN_FRONTEND=noninteractive
    # Cache signed repository downloads between disposable checks, without retaining installed packages.
    rm -f /etc/apt/apt.conf.d/docker-clean
    # Minimal OCI images exclude man pages; retain this package's documentation for the layout checks.
    printf 'path-include=/usr/share/man/\npath-include=/usr/share/man/man1/\npath-include=/usr/share/man/man1/optchat.1.gz\npath-include=/usr/share/doc/optchat/\npath-include=/usr/share/doc/optchat/*\n' > /etc/dpkg/dpkg.cfg.d/zz-optchat-documentation
    expected=amd64; [ "$ARCH" != aarch64 ] || expected=arm64
    [ "$(dpkg-deb -f /packages/current.deb Architecture)" = "$expected" ]
    if [ -n "$PREVIOUS" ]; then
      [ "$(dpkg-deb -f /packages/previous.deb Architecture)" = "$expected" ]
      dpkg --compare-versions "$(dpkg-deb -f /packages/current.deb Version)" gt "$(dpkg-deb -f /packages/previous.deb Version)"
    fi
    apt-get update
    if [ -n "$PREVIOUS" ]; then apt-get install -y /packages/previous.deb; fi
    install_current() { apt-get install -y /packages/current.deb; }
    remove_current() { apt-get purge -y optchat; }
    installed() { dpkg-query -W -f='${db:Status-Status}' optchat 2>/dev/null | grep -qx installed; }
    payload=/usr/lib/optchat/optchat
    ;;
  rpm)
    # Retry sustained slow mirrors and use the shared cache when rehearsing full desktop installs.
    printf '\nmax_parallel_downloads=8\nminrate=20000\ntimeout=30\n' >> /etc/dnf/dnf.conf
    [ "$(rpm -qp --qf '%{ARCH}' /packages/current.rpm)" = "$ARCH" ]
    if [ -n "$PREVIOUS" ]; then
      [ "$(rpm -qp --qf '%{ARCH}' /packages/previous.rpm)" = "$ARCH" ]
      previous="$(rpm -qp --qf '%{VERSION}-%{RELEASE}' /packages/previous.rpm)"
      current="$(rpm -qp --qf '%{VERSION}-%{RELEASE}' /packages/current.rpm)"
      [ "$previous" != "$current" ] || { echo "Upgrade requires a different previous version/release." >&2; exit 1; }
      # Order is checked by the actual DNF upgrade transaction below.
      dnf --setopt=tsflags= --setopt=keepcache=True install -y /packages/previous.rpm
    fi
    # Minimal Fedora images set nodocs; exercise the normal desktop package layout.
    install_current() { dnf --setopt=tsflags= --setopt=keepcache=True install -y /packages/current.rpm; }
    remove_current() { dnf remove -y optchat; }
    installed() { rpm -q optchat >/dev/null 2>&1; }
    payload=/usr/libexec/optchat/optchat
    ;;
  *) exit 2 ;;
esac
mkdir -p "$HOME/.config/optchat"
printf 'optchat-package-preservation-test\n' > "$HOME/.config/optchat/package-test-sentinel"
if [ -n "$PREVIOUS" ] && [ "$FORMAT" = rpm ]; then
  dnf --setopt=tsflags= --setopt=keepcache=True upgrade -y /packages/current.rpm
  [ "$(rpm -q --qf '%{VERSION}-%{RELEASE}' optchat)" = "$current" ]
else
  install_current
fi
installed
[ -x /usr/bin/optchat ] && [ "$(readlink -f /usr/bin/optchat)" = "$payload" ]
[ "$(stat -c '%a:%U:%G' "$payload")" = 755:root:root ]
[ -f /usr/share/applications/com.reframe.optchat.gpui.desktop ]
[ -f /usr/share/icons/hicolor/512x512/apps/com.reframe.optchat.gpui.png ]
[ -f /usr/share/man/man1/optchat.1.gz ]
[ "$(stat -c '%a:%U:%G' /usr/share/man/man1/optchat.1.gz)" = 644:root:root ]
ldd "$payload" > /tmp/optchat-ldd.txt
cat /tmp/optchat-ldd.txt
if grep -q 'not found' /tmp/optchat-ldd.txt; then echo "Unresolved shared library." >&2; exit 1; fi
grep -qx optchat-package-preservation-test "$HOME/.config/optchat/package-test-sentinel"
remove_current
if installed; then echo "Package still installed after removal." >&2; exit 1; fi
for file in "$payload" /usr/bin/optchat /usr/share/applications/com.reframe.optchat.gpui.desktop /usr/share/icons/hicolor/512x512/apps/com.reframe.optchat.gpui.png /usr/share/man/man1/optchat.1.gz; do
  [ ! -e "$file" ] && [ ! -L "$file" ] || { echo "Package file remains: $file" >&2; exit 1; }
done
grep -qx optchat-package-preservation-test "$HOME/.config/optchat/package-test-sentinel"
install_current
installed
grep -qx optchat-package-preservation-test "$HOME/.config/optchat/package-test-sentinel"
echo "PASS: dependencies, install, file layout, GLIBC/shared libraries, removal/purge, data preservation, reinstall."
[ -z "$PREVIOUS" ] || echo "PASS: package manager upgrade from the supplied previous package."
