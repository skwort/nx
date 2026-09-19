{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.nx;
  defaultPackage = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
  toml = pkgs.formats.toml { };
  settings = {
    system = {
      inherit (cfg) flake configuration;
    };
    daemon = {
      check_interval = cfg.checkInterval;
      startup_delay = cfg.startupDelay;
    };
    notifications = {
      enabled = cfg.notifications.enable;
    }
    // lib.optionalAttrs (cfg.notifications.viewCommand != [ ]) {
      view_command = cfg.notifications.viewCommand;
    };
  };
  configFile = toml.generate "nx-config.toml" settings;
  configHome = pkgs.runCommand "nx-config" { } ''
    mkdir -p "$out/nx"
    ln -s ${configFile} "$out/nx/config.toml"
  '';
  configuredPackage = pkgs.symlinkJoin {
    name = "nx-configured";
    paths = [ cfg.package ];
    nativeBuildInputs = [ pkgs.makeWrapper ];
    postBuild = ''
      for program in nx nxd; do
        wrapProgram "$out/bin/$program" --set XDG_CONFIG_HOME ${configHome}
      done
    '';
  };
in
{
  options.services.nx = {
    enable = lib.mkEnableOption "the nx NixOS update helper";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage;
      defaultText = lib.literalExpression "inputs.nx.packages.\${pkgs.system}.default";
      description = "The nx package to install and run.";
    };

    flake = lib.mkOption {
      type = lib.types.str;
      example = "/home/alice/nixos-config";
      description = "Path to the NixOS configuration flake to check.";
    };

    configuration = lib.mkOption {
      type = lib.types.str;
      default = config.networking.hostName;
      defaultText = lib.literalExpression "config.networking.hostName";
      description = "Name under nixosConfigurations to evaluate.";
    };

    checkInterval = lib.mkOption {
      type = lib.types.str;
      default = "6h";
      description = "Time between completed scheduled checks.";
    };

    startupDelay = lib.mkOption {
      type = lib.types.str;
      default = "60s";
      description = "Delay after daemon startup before the first check.";
    };

    notifications = {
      enable = lib.mkEnableOption "desktop notifications" // {
        default = true;
      };

      viewCommand = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [
          "${pkgs.kitty}/bin/kitty"
          "--hold"
          "nx"
          "update"
          "list"
        ];
        description = "Command launched by the notification's View changes action.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ configuredPackage ];

    systemd.user.services.nxd = {
      description = "nx NixOS update daemon";
      wantedBy = [ "default.target" ];
      after = [ "graphical-session.target" ];
      path = [ configuredPackage ];
      serviceConfig = {
        ExecStart = "${configuredPackage}/bin/nxd";
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
