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
      ntfy = {
        enabled = cfg.notifications.ntfy.enable;
        mode = cfg.notifications.ntfy.mode;
      }
      // lib.optionalAttrs (cfg.notifications.ntfy.topicUrl != null) {
        topic_url = cfg.notifications.ntfy.topicUrl;
      }
      // lib.optionalAttrs (cfg.notifications.ntfy.tokenFile != null) {
        token_file = cfg.notifications.ntfy.tokenFile;
      }
      // lib.optionalAttrs (cfg.notifications.ntfy.tokenEnv != null) {
        token_env = cfg.notifications.ntfy.tokenEnv;
      };
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
        default = [ ];
        description = "Command launched by the notification's View changes action.";
      };

      ntfy = {
        enable = lib.mkEnableOption "ntfy update notifications";
        topicUrl = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "https://ntfy.example.com/nixos-updates";
          description = "Full URL of the ntfy topic.";
        };
        mode = lib.mkOption {
          type = lib.types.enum [ "summary" "detailed" ];
          default = "summary";
          description = "Send a compact message or attach the full report.";
        };
        tokenFile = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "/run/secrets/nx-ntfy-token";
          description = "Absolute path to a runtime token file; contents are read by nxd.";
        };
        tokenEnv = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "NX_NTFY_TOKEN";
          description = "Name of an environment variable containing the ntfy token in the nxd service.";
        };
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !cfg.notifications.ntfy.enable || cfg.notifications.ntfy.topicUrl != null;
        message = "services.nx.notifications.ntfy.topicUrl is required when ntfy is enabled";
      }
      {
        assertion = cfg.notifications.ntfy.tokenFile == null || cfg.notifications.ntfy.tokenEnv == null;
        message = "configure only one ntfy token source";
      }
    ];
    environment.systemPackages = [ configuredPackage ];

    systemd.user.services.nxd = {
      description = "nx NixOS update daemon";
      wantedBy = [ "default.target" ];
      after = [ "graphical-session.target" ];
      path = [ configuredPackage pkgs.less pkgs.xdg-terminal-exec ];
      serviceConfig = {
        ExecStart = "${configuredPackage}/bin/nxd";
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
