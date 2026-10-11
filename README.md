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
seconds after startup and checks every six hours. Desktop notifications are
enabled by default; their View changes action uses `xdg-terminal-exec` to open
your preferred terminal. Set `services.nx.notifications.viewCommand` to a list
of command arguments if your terminal needs a custom launcher, or set
`services.nx.notifications.enable = false` to disable desktop delivery.

To also publish updates to a self-hosted ntfy topic:

```nix
services.nx.notifications.ntfy = {
  enable = true;
  topicUrl = "https://ntfy.example.com/nixos-updates";
  tokenFile = "/run/secrets/nx-ntfy-token";
  mode = "summary"; # or "detailed"
};
```

`tokenFile` is a runtime path string, suitable for a sops-managed secret. The
file must be readable by the user running `nxd`. Alternatively, use
`tokenEnv = "NX_NTFY_TOKEN"` and provide that variable to the systemd user
service (for example with an `EnvironmentFile` override); shell variables are
not automatically inherited by user services. Configure only one token source.
The token is read when a notification is sent. Detailed mode sends the summary
as notification text and attaches the full plain-text report; attachment size
and expiry depend on the ntfy server. The report remains available through
`nx update list`.

For direct `config.toml` configuration, use `[notifications.ntfy]` with
`enabled`, `topic_url`, `mode`, and either `token_file` or `token_env`. Existing
`[notifications]` `enabled` and `view_command` settings still control desktop
notifications. The two channels deliver independently. Successful deliveries
are remembered across daemon restarts per channel and topic. New revisions
produce a new alert; checks with no updates reset duplicate suppression.

## Usage

```sh
nx update check     # run a fresh check
nx update list      # show the latest cached report
systemctl --user status nxd
```

Interactive reports open in `$PAGER` or `less -R`. Use `--no-pager` to print
directly. `nx notification test` sends through enabled channels using the latest
cached report, without contacting the daemon or changing duplicate suppression.
Pass `--flake` and `--configuration` when they are not set in `config.toml`.
Only scheduled checks send automatic notifications.

The package can also be run directly:

```sh
nix run github:skwort/nx -- --help
```
