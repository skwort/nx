# nx

`nx` is a helper application for NixOS. It checks what a flake update would
change, reports kernel, input, and declared package updates, and leaves the
original configuration untouched.

## NixOS module

Add the flake input:

```nix
inputs.nx.url = "github:skwort/nx";
```

Import `nx.nixosModules.default` in your `nixosSystem` modules, then enable it:

```nix
services.nx = {
  enable = true;
  flake = "/home/alice/nixos-config";
};
```

The configuration name defaults to the system hostname. The daemon waits 60
seconds after startup, checks every six hours, and sends desktop notifications.
These defaults and the notification view command are configurable through the
module.

## Usage

```sh
nx update check     # run a fresh check
nx update list      # show the latest cached report
systemctl --user status nxd
```

Interactive reports open in `$PAGER` or `less -R`. Use `--no-pager` to print
directly. In a debug build, `nx notification test` sends a notification for the
latest cached report immediately, without contacting the daemon; its View
changes action opens that same debug binary in Kitty. Pass `--flake` and
`--configuration` when they are not set in `config.toml`.

The package can also be run directly:

```sh
nix run github:skwort/nx -- --help
```
