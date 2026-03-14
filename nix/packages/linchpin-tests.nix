{
  stdenv,
  rust,
  jq,
  linchpin,
}:

linchpin.overrideAttrs (
  finalAttrs: previousAttrs: {
    pname = "linchpin-tests";

    nativeBuildInputs = previousAttrs.nativeBuildInputs or [ ] ++ [ jq ];

    # Mostly taken from cargoBuildHook but only builds the test binary
    postBuild = ''
      (
      set -x
      ${rust.envVars.setEnv} cargo test '*' --no-run "''${flagsArray[@]}" -- --ignored
      )
    '';

    postInstall = previousAttrs.postInstall + ''
      find /build/source/target/${stdenv.targetPlatform.rust.rustcTarget}/release/deps/ \
        -name "integration-*" \
        -type f \
        -executable \
        -execdir install -D {} $out/bin/linchpin-tests \;
    '';

    doCheck = false;

    meta.mainProgram = "linchpin-tests";
  }
)
