{
  config,
  lib,
  pkgs,
  ...
}:
{
  options = {
    services.reproducibility-automation = {
      enable = lib.mkEnableOption "enable tracking server";

      openFirewall = lib.mkEnableOption "open port in firewall";
      socket-ip = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        example = "0.0.0.0";
        description = ''
          Socket IP to listen for rebuild information.
        '';
      };
      port = lib.mkOption {
        type = lib.types.port;
        default = 8080;
        description = ''
          Port to listen for rebuild information (http).
        '';
      };

      dataDir = lib.mkOption {
        type = lib.types.path;
        default = "/var/lib/reproducibility-automation";
        description = ''
          Parent Directory to derive other filesystem related options. You probably only need to edit this path if any at all.
        '';
      };
      db-file = lib.mkOption {
        type = lib.types.path;
        default = "${config.services.reproducibility-automation.dataDir}/server.db";
        description = ''
          Filesystem path for a sqlite database storing a store derivations build reproducibility status.
        '';
      };
      savefile-path = lib.mkOption {
        type = lib.types.path;
        default = "${config.services.reproducibility-automation.dataDir}/savefile.json";
        description = ''
          Filesystem path to store reports from the waitlist/queue. If persistent-reports option is set the content will be used to resume after restarting the service.
        '';
      };
      gc-links-path = lib.mkOption {
        type = lib.types.path;
        default = "${config.services.reproducibility-automation.dataDir}/gc-roots";
        example = "/var/lib/reproducibility-automation/gc-roots";
        description = ''
          Filesystem path to a directory to create symlinks against store derivations. Protects needed store derivations against automated garbadge collection.
          Symlinks are removed upon test completion.
        '';
      };
      savefile-history-path = lib.mkOption {
        type = lib.types.path;
        default = "${config.services.reproducibility-automation.dataDir}/comment-history.json";
        description = ''
          Filesystem path to store specifics from already posted reports. In case a singular pipeline has multiple reports requested the incomplete comments will be edited to prevent creating additional comments.
        '';
      };

      nix-store = lib.mkOption {
        type = lib.types.str;
        default = "ssh-ng://localhost";
        example = "ssh-ng://remote.machine.com";
        description = ''
          Nix store used to (build and) rebuild derivations.

          Using nix-build with a remote machine as builder could look like this:
          `nix-build /nix/store/j73r5pf6m6x0kc53czn353bk2k2hxcds-mesa-24.2.8.drv --check --max-jobs 0 --eval-store auto --store ssh-ng://remote-builder.internal`
        '';
      };
      gitlab-url = lib.mkOption {
        type = lib.types.str;
        default = "";
        example = "https://git.domain.com";
        description = ''
          Gitlab instance where the merge request comments with the test results will be posted on.
        '';
      };
      gitlab-token-file = lib.mkOption {
        type = lib.types.path;
        default = "";
        example = "/run/secrets/my-secret-gitlab-token";
        description = ''
          Gitlab token allowing to post merge request comments.
        '';
      };

      persistent-reports = lib.mkEnableOption "When restarting the service, resume the left over reports instead of doing a reset";
      simultaneous-builds = lib.mkOption {
        type = lib.types.int;
        default = 1;
        example = 2;
        description = ''
          Numer of simultaneously run nix-build commands
        '';
      };
      max-rebuild-tries = lib.mkOption {
        type = lib.types.int;
        default = 3;
        example = 7;
        description = ''
          How often a rebuild is done (on repeated reports) before taking the past test results at face value.
        '';
      };
    };
  };
  config = lib.mkIf config.services.reproducibility-automation.enable {
    systemd.services.reproducibility-automation = {
      enable = true;
      description = "server side for tracking already rebuilt derivations";
      path = [
        pkgs.nix
      ];
      after = [ "network.target" ];
      serviceConfig = {
        Type = "exec";
        ExecStart = "${
          pkgs.callPackage ./reproducibility-automation.nix { }
        }/bin/reproducibility-automation --db-file ${config.services.reproducibility-automation.db-file} --socket-address ${config.services.reproducibility-automation.socket-ip}:${builtins.toString config.services.reproducibility-automation.port} --nix-store ${config.services.reproducibility-automation.nix-store} --gitlab-url ${config.services.reproducibility-automation.gitlab-url} --simultaneous-builds ${builtins.toString config.services.reproducibility-automation.simultaneous-builds} --gc-links-path ${config.services.reproducibility-automation.gc-links-path} ${lib.optionalString config.services.reproducibility-automation.persistent-reports "--persistent-reports"} --savefile-path ${config.services.reproducibility-automation.savefile-path} --savefile-history-path ${config.services.reproducibility-automation.savefile-history-path} --max-rebuild-tries ${builtins.toString config.services.reproducibility-automation.max-rebuild-tries}";
        WatchdogSec = "1min";
        Restart = "always";
        RestartSec = 20;
        LoadCredential = [ "gitlab_token:${config.services.reproducibility-automation.gitlab-token-file}" ];
      };
      wantedBy = [ "multi-user.target" ];
    };

    networking.firewall.allowedTCPPorts =
      lib.mkIf config.services.reproducibility-automation.openFirewall
        [ config.services.reproducibility-automation.port ];
  };
}
