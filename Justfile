#!/usr/bin/env just --justfile
name := 'wlrix-greeter'
export APPID := 'com.wlrix.greeter'

rootdir := ''
prefix := '/usr'

# Which distribution's PAM stack to install. `arch` uses the system-login includes, `debian`
# the common-* ones; the two are not interchangeable, and installing the wrong one leaves a
# stack whose includes do not exist, which is a login that cannot succeed.
#
# Not detected at install time on purpose: a package build stages into DESTDIR on whatever
# machine the builder happens to be, which says nothing about where the package will land.
pam-flavor := env("PAM_FLAVOR", "arch")

base-dir := absolute_path(clean(rootdir / prefix))

bin-dir := base-dir / 'bin'
# Systemd sysusers/tmpfiles components directories
lib-dir := base-dir / 'lib'
# PAM and greetd read from /etc, which is not under the prefix wherever the prefix is.
etc-dir := absolute_path(clean(rootdir / 'etc'))

bin-src := 'target' / 'release' / name
bin-dst := bin-dir / name

# sysusers.d
sysusers-src := 'setup' / name + '.sysusers'
sysusers-dst := lib-dir / 'sysusers.d' / name + '.conf'
# tmpfiles.d
tmpfiles-src := 'setup' / name + '.tmpfiles'
tmpfiles-dst := lib-dir / 'tmpfiles.d' / name + '.conf'
# systemd unit. This *is* greetd, started against the wlRIX configuration, so enabling it
# replaces the distribution's greetd.service rather than joining it.
service-src := 'setup' / name + '.service'
service-dst := lib-dir / 'systemd' / 'system' / name + '.service'

start-src := name + '-start.sh'
start-dst := bin-dir / name + '-start'

# The stack that authenticates whoever logs in, and the one for the greeter's own session.
pam-src := 'setup' / name + '.pam.' + pam-flavor
pam-dst := etc-dir / 'pam.d' / name
greeter-pam-src := 'setup' / name + '-greeter.pam.' + pam-flavor
greeter-pam-dst := etc-dir / 'pam.d' / name + '-greeter'

greetd-src := name + '.toml'
greetd-dst := etc-dir / 'greetd' / name + '.toml'

default:
  @just --list

release:
  cargo build --release

lint:
  cargo clippy

test:
  cargo test

# Draw the login dialog with no greetd behind it, for eyeballing against the reference.
# Takes a state: selected, typed, password, error. See examples/paint_preview.rs.
[doc("Draw the login dialog with no greetd behind it")]
preview state='':
  cargo run --example paint_preview -- {{state}}

# A stand-in greetd that authenticates nobody, to run the greeter against. See
# examples/fake_greetd.rs.
[doc("Run a stand-in greetd that authenticates nobody")]
fake-greetd sock='/tmp/fake-greetd.sock' password='':
  cargo run --example fake_greetd -- {{sock}} {{password}}

# Install the greeter, its launcher, and everything the system needs to run it as its own user.
#
# Deliberately does not build. It is normally run as root, and building as root leaves a target
# directory nobody can write to afterwards:
#
#     just release && sudo just install
[doc("Install the greeter and its setup files (build first; run as root)")]
install:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ ! -x '{{bin-src}}' ]; then
      echo "no release build -- run 'just release' first" >&2
      exit 1
  fi
  install -Dm0755 '{{bin-src}}' '{{bin-dst}}'
  install -Dm0755 '{{start-src}}' '{{start-dst}}'
  install -Dm0644 '{{sysusers-src}}' '{{sysusers-dst}}'
  install -Dm0644 '{{tmpfiles-src}}' '{{tmpfiles-dst}}'
  install -Dm0644 '{{service-src}}' '{{service-dst}}'
  # 0644, as PAM wants: root-owned and world-readable, never writable by anyone else.
  install -Dm0644 '{{pam-src}}' '{{pam-dst}}'
  install -Dm0644 '{{greeter-pam-src}}' '{{greeter-pam-dst}}'
  # Beside greetd's own config rather than over it: config.toml belongs to the administrator,
  # and may already be pointing at another greeter.
  install -Dm0644 '{{greetd-src}}' '{{greetd-dst}}'
  for f in '{{bin-dst}}' '{{start-dst}}' '{{sysusers-dst}}' '{{tmpfiles-dst}}' \
           '{{service-dst}}' '{{pam-dst}}' '{{greeter-pam-dst}}' '{{greetd-dst}}'; do
      echo "installed $f"
  done
  echo
  echo "PAM stack: {{pam-flavor}}. Set pam-flavor= to change it."
  echo
  echo "Create the account and its directories:"
  echo
  echo "    systemd-sysusers && systemd-tmpfiles --create"
  echo
  echo "Then make it the display manager. {{name}}.service is greetd started"
  echo "against the wlRIX config, so it replaces the distribution's unit:"
  echo
  echo "    systemctl disable greetd.service"
  echo "    systemctl enable {{name}}.service"
  echo
  echo "wlrix-greeter-start runs wlrix-compositor, which must be on PATH."

# Remove what `install` put down. Leaves the account, its home directory and greetd's own
# configuration alone: the first two may hold state, and the last is not ours.
[doc("Remove what install put down")]
uninstall:
  #!/usr/bin/env bash
  set -euo pipefail
  rm -f '{{bin-dst}}' '{{start-dst}}' '{{sysusers-dst}}' '{{tmpfiles-dst}}' \
        '{{service-dst}}' '{{pam-dst}}' '{{greeter-pam-dst}}' '{{greetd-dst}}'
  echo "removed the greeter and its setup files"
  echo "the wlrix-greeter account and /var/lib/wlrix-greeter are left alone"

clean:
  cargo clean
