{ lib, inputs, ... }:
{
  perSystem = { pkgs, ... }:
    let
      inherit (pkgs) craneLib;
      inherit (pkgs._nix-ninja) cargoArtifacts commonArgs src;

    in {
      checks = {
        # Run clippy (and deny all warnings) on the crate source,
        # again, reusing the dependency artifacts from above.
        #
        # Note that this is done as a separate derivation so that
        # we can block the CI if there are issues here, but not
        # prevent downstream consumers from building our crate by itself.
        nix-ninja-clippy = craneLib.cargoClippy (commonArgs // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets -- --deny warnings";
        });

        nix-ninja-doc = craneLib.cargoDoc (commonArgs // {
          inherit cargoArtifacts;
        });

        # Check formatting
        nix-ninja-fmt = craneLib.cargoFmt {
          inherit src;
        };

        nix-ninja-toml-fmt = craneLib.taploFmt {
          src = lib.fileset.toSource {
            root = ../../.;
            fileset = inputs.globset.lib.globs ../../. [
              "**/*.toml"
            ];
          };

          # taplo arguments can be further customized below as needed
          # taploExtraArgs = "--config ./taplo.toml";
        };

        # Audit dependencies
        nix-ninja-audit = craneLib.cargoAudit {
          inherit src;
          inherit (inputs) advisory-db;
        };

        # Audit licenses
        nix-ninja-deny = craneLib.cargoDeny {
          src = lib.fileset.toSource {
            root = ../../.;
            fileset = inputs.globset.lib.globs ../../. [
              "deny.toml"
              "Cargo.lock"
              "**/Cargo.toml"
              "**/*.rs"
            ];
          };
        };

        # Run tests with cargo-nextest
        # Consider setting `doCheck = false` on `nix-ninja` if you do not want
        # the tests to run twice
        nix-ninja-nextest = craneLib.cargoNextest (commonArgs // {
          inherit cargoArtifacts;
          partitions = 1;
          partitionType = "count";
          cargoNextestPartitionsExtraArgs = "--no-tests=pass";
        });
      };
    };
}
