#!/bin/sh
# The login screen's own settings -- keyboard, color scheme, displays -- are in greeter.toml,
# written by the Login Settings panel. Until somebody writes it, the compositor falls back to
# the usual /etc/wlrix/compositor.toml.
exec wlrix-compositor --config /etc/wlrix/greeter.toml -c wlrix-greeter > /dev/null 2>&1
