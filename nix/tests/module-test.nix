{
  name = "sanity check module";
  defaults = {

  };
  nodes =
    let
      baseConfig = {
        security.pam.services.sshd.allowNullPassword = true;
        services.openssh = {
          enable = true;
          settings = {
            PermitRootLogin = "yes";
            PermitEmptyPasswords = "yes";
          };
        };
        virtualisation.graphics = false;
      };
    in
    {
      "server" =
        { self, ... }:
        {
          imports = [ baseConfig ];
          virtualisation.forwardPorts = [
            {
              from = "host";
              host.port = 2000;
              guest.port = 22;
            }
          ];

          # package module
          services.linchpin = {
            enable = true;
            openFirewall = true;
            socket-ip = "0.0.0.0";
            port = 80;
            gitlab.enable = true;
            gitlab.url = "https://gitlab.noi0103.com";
            gitlab.token = "/etc/gitlab_token";
            max-rebuild-tries = 1;
            persistent-reports = true;
            simultaneous-builds = 4;
          };
          environment.etc."gitlab_token".text = "empty-token";

        };

      "client" =
        { pkgs, ... }:
        {

          imports = [ baseConfig ];
          virtualisation.forwardPorts = [
            {
              from = "host";
              host.port = 2001;
              guest.port = 22;
            }
          ];

          # tooling
          environment.systemPackages = [
            pkgs.curl
            #pkgs.getclosure
          ];
        };

    };
  testScript = ''
    start_all()
    server.wait_for_unit("multi-user.target")
    client.wait_for_unit("multi-user.target")

    server.wait_for_unit("linchpin.service")
    server.wait_for_open_port(80)

    server.succeed("curl --silent http://127.0.0.1:80/ping")

    response = client.succeed("curl --silent http://server:80/ping")
    assert "reports in waitlist:" in response
    print(response)
  '';
}
