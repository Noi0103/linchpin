A service to rebuild every element of a Nix build closures sent to it and report the results as a GitLab merge request comment.

Reproducibility for software builds is considered relevant for software security. Reproducible builds aim to reduce the chances a supply chain attack is successful. Such an attack only needs a single vulnerability.

```nix
"${linchpin.outPath}/nix/module.nix"
{
  services.linchpin = {
    enable = false;
    openFirewall = false;
    socket-ip = "127.0.0.1";
    port = 8080;
    gitlab-url = "https://gitlab.noi0103.com";
    gitlab-token-file = "/etc/gitlab_token";
    max-rebuild-tries = 1;
    persistent-reports = false;
  };
  environment.etc."gitlab_token".text = "empty-token";
  environment.systemPackages = [ linchpin.packages."x86_64-linux".getclosure ];
}
```

# Table of contents
- [usage example configuration](#usage-example-configuration)
- [REST endpoints](#rest-endpoints)
- [making a package build reproducible](#making-a-package-build-reproducible)
  - [examples to find and make builds reproducible](#examples-to-find-and-make-builds-reproducible)
    - [example 1](#example-1)
    - [example 2](#example-2)
- [tips and notes](#tips-and-notes)
- [why linchpin](#why-linchpin)


# usage example configuration
Singular machine setup example:

`flake.nix`:
```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.05";

    linchpin.url = "github:Noi0103/linchpin.git";
  };

  outputs = {
    self,
    nixpkgs,
    linchpin,
    ...
  }@inputs:
  {
    nixosConfigurations = {
      machine = nixpkgs.lib.nixosSystem {
        specialArgs = {inherit inputs;};
        modules = [
          ./configuration.nix

          linchpin.nixosModules.linchpin
          {
            environment.systemPackages =
              [ inputs.linchpin.outputs.packages.x86_64-linux.getclosure ];

            services.linchpin = {
              enable = true;
              openFirewall = false;
              db-file = "/var/lib/linchpin/server.db";
              socket-ip = "0.0.0.0";
              port = 8080;
            };
          }
        ];
      };
    };
  };
}
```

# REST endpoints:
## /ping
`/ping` check the health of a MutexGuard shared state and return the number of waiting reports held in it

## /report
`/report` accept a multipart http request to test a full build closure

## /metrics
openmetrics/prometheus compatible metrics source

# making a package build reproducible
1. update your project fork to see most recent report (in case of upstream fixes for older reported derivations)

2. get a derivation path from the merge request comment created by the testing service
3. verify it is non-reproducible and check the diffoscope
4. check if it is upstream issue or exclusive to the local git (test the nixpkgs version)
	- if issue also on nixpkgs -> github issue and pull request on nixpkgs
	- else -> local package definition should be the culprit
5. find package definition
6. check file edits/commits to upstream (is pkgs definition different between master/unstable/stable; meaning the fix might already be on its way and is only waiting for hydra for example)

# tips and notes
## nix closures: parents/upstream
find what depends on the derivation `myderivation.drv`

one level:
```sh
nix-store --query --referrers /nix/store/myderivation.drv
```

everything:
```sh
nix-store --query --referrers-closure /nix/store/myderivation.drv
```

## nix closures: children/downstream
easiest to use and interactive is
```sh
nix-shell -p nix-tree --run "nix-tree /nix/store/myderivation.drv"
```

one level:
```sh
nix-store --query --references /nix/store/myderivation.drv
```

everything:
```sh
nix-store --query --requisites /nix/store/myderivation.drv
```

make it a colorful image
```sh
nix-shell -p graphviz --run "nix-store --query --graph /nix/store/lri77scxpmyliswy8caq7si8ps8kxy1a-cargo-vendor-dir.drv > tmp.dot && dot -Tsvg tmp.dot -o out.svg && rm tmp.dot"
```
