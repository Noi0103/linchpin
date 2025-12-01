{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    pre-commit-hooks.url = "github:cachix/git-hooks.nix";
  };

  outputs =
    {
      self,
      nixpkgs,
      pre-commit-hooks,
      ...
    }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};

      supportedSystems = [ "x86_64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems (system: f system);
      nixpkgsFor = forAllSystems (
        system:
        import nixpkgs {
          inherit system;
          overlays = [ self.overlays.default ];
        }
      );
    in
    {
      overlays.default = final: prev: {
        linchpin = final.callPackage ./linchpin.nix { };
        getclosure = final.writeShellScriptBin "getclosure" (builtins.readFile ./getclosure.sh);
      };

      packages = forAllSystems (system: {
        inherit (nixpkgsFor.${system}) linchpin getclosure;

        default = self.packages.${system}.linchpin;

        stable = pkgs.runCommand "stable" { } "echo 5432 > $out";
        unstable = pkgs.runCommand "unstable" { } "echo $RANDOM > $out";
        available = pkgs.fetchurl {
          url = "https://github.com/fluidicon.png";
          hash = "sha256-G+3WoZSJcfB5cEFHFwElA4BTCfJa8LLFQtvDUktYgOk=";
        };
        unavailable = pkgs.fetchurl {
          url = "https://github.com/fluidiconHIIAMBREAKINGSTUFF.png";
          hash = "sha256-G+3WoZSJcfB5cEFHFwElA4BTCfJa8LLFQtvDUktYgOk=";
        };
        hashmismatch = pkgs.fetchurl {
          url = "https://github.com/fluidicon.png";
          hash = "sha256-G+3WoZSJcfB5cEFHFwElA4BTCHIIAMBREAKINGSTUFF=";
        };
      });

      nixosModules.default = self.nixosModules.linchpin;
      nixosModules.linchpin = ./module.nix;

      devShells.${system}.default = pkgs.mkShell {
        inherit (self.checks.${system}.pre-commit-check) shellHook;
        buildInputs = self.checks.${system}.pre-commit-check.enabledPackages;

        packages = with pkgs; [
          nixpkgs-fmt
          shellcheck

          # rust
          rustc
          cargo
          rust-analyzer
          rustfmt
          clippy

          # If the dependencies need system libs, you usually need pkg-config + the lib
          pkg-config
          openssl

          # other
          jq
          sqlite
        ];

        env = {
          RUST_BACKTRACE = "full";

          # gitlab ci shell runner environment
          CI_MERGE_REQUEST_PROJECT_ID = "1229";
          CI_MERGE_REQUEST_IID = "22";
          CI_COMMIT_SHA = "000";
          CI_JOB_NAME = "no_name";
          CI_PIPELINE_ID = "0";
        };
      };

      checks = forAllSystems (
        system: with nixpkgsFor.${system}; {
          inherit (self.packages.${system}) linchpin getclosure;

          # interactive:
          # ssh -o "UserKnownHostsFile=/dev/null" -o "StrictHostKeyChecking=no" root@localhost -p 2000
          vmTest = pkgs.testers.runNixOSTest {
            name = "report-stable";
            nodes = {
              # logs will show and ignore a failure to create a merge request, that is intended
              "server" =
                { ... }:
                {
                  # debug interactive via ssh
                  services.openssh = {
                    enable = true;
                    settings = {
                      PermitRootLogin = "yes";
                      PermitEmptyPasswords = "yes";
                    };
                  };
                  security.pam.services.sshd.allowNullPassword = true;
                  virtualisation.forwardPorts = [
                    {
                      from = "host";
                      host.port = 2000;
                      guest.port = 22;
                    }
                  ];

                  # stuff
                  virtualisation.graphics = false;

                  # package module
                  imports = [ self.nixosModules.linchpin ];
                  services.linchpin = {
                    enable = true;
                    openFirewall = true;
                    socket-ip = "0.0.0.0";
                    port = 80;
                    gitlab-url = "https://gitlab.of-some-domain.com";
                    gitlab-token-file = "/etc/gitlab_token";
                    max-rebuild-tries = 1;
                  };
                  environment.etc."gitlab_token".text = "empty-token";
                  environment.etc."stable.db" = {
                    mode = "0666";
                    source = ./tests/stable-database;
                  };
                };
              "client" =
                { ... }:
                {
                  # debug interactive via ssh
                  services.openssh = {
                    enable = true;
                    settings = {
                      PermitRootLogin = "yes";
                      PermitEmptyPasswords = "yes";
                    };
                  };
                  security.pam.services.sshd.allowNullPassword = true;
                  virtualisation.forwardPorts = [
                    {
                      from = "host";
                      host.port = 2001;
                      guest.port = 22;
                    }
                  ];

                  # stuff
                  virtualisation.graphics = false;

                  # tooling
                  environment.systemPackages = [
                    pkgs.curl
                  ];

                  # sending a prepared report
                  environment.etc."closure-paths.json" = {
                    source = ./tests/stable-closure-paths.json;
                  };
                  environment.etc."nix-export" = {
                    source = ./tests/stable-nix-export;
                  };
                };
            };
            testScript =
              { nodes, ... }:
              ''
                start_all()
                server.wait_for_unit("multi-user.target")
                client.wait_for_unit("multi-user.target")

                server.succeed("cp /etc/stable.db /var/lib/linchpin/server.db")
                server.wait_for_unit("linchpin.service")
                server.wait_for_open_port(80)

                server.succeed("curl --silent http://127.0.0.1/metrics")
                server.succeed("curl --silent http://127.0.0.1/ping")

                response = client.succeed("curl --fail --silent --verbose http://server/ping")
                assert "reports in waitlist:" in response
                response = client.succeed("curl --fail --silent --verbose http://server/metrics")
                assert "# HELP linchpin_axum_requests Count of requests." in response

                client.succeed('curl -s -X POST -F "json=@/etc/closure-paths.json" -F "closure=@/etc/nix-export" "http://server/report"')
                client.wait_until_succeeds("curl --silent http://server/ping | grep -q 'reports in waitlist: 0'")
              '';
          };

          # ref https://github.com/cachix/git-hooks.nix#installation--usage
          pre-commit-check = pre-commit-hooks.lib.${system}.run {
            src = ./.;
            hooks = {
              nixfmt-rfc-style.enable = true;
              deadnix.enable = false;
              shellcheck = {
                enable = true;
                args = [
                  "-e"
                  "SC2148"
                  "-e"
                  "SC2086"
                  "-e"
                  "SC2016"
                ];
              };
              rustfmt.enable = true;
              clippy = {
                enable = true;
                packageOverrides.cargo = pkgs.cargo;
                packageOverrides.clippy = pkgs.clippy;
                settings.allFeatures = true;
                extraPackages = with pkgs; [
                  pkg-config
                  openssl
                ];
              };
            };
            settings = {
              rust.check.cargoDeps = pkgs.rustPlatform.importCargoLock {
                lockFile = ./Cargo.lock;
              };
            };
          };
        }
      );
    };
}
