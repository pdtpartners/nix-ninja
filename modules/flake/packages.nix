{ self, ... }:
{
  perSystem = { pkgs, system, ... }: {
    packages = {
      inherit (pkgs)
        nix-ninja
        nix-ninja-task
        nix-ninja-llvm-coverage
        nixDdPortable
      ;

      default = pkgs.nix-ninja;

      example-hello = pkgs.example-hello.target;
      example-header = pkgs.example-header.target;
      example-incremental = pkgs.example-incremental.target;
      example-nix = pkgs.example-nix.target;
    };

    devShells.default = pkgs.craneLib.devShell {
      checks = self.checks.${system};

      packages = with pkgs; [
        agg
        gnumake
        just
        meson
        nixDdPortable
        taplo
      ];
      
      shellHook = ''
        if command -v direnv_layout_dir 2>&1 >/dev/null; then
          export NP_LOCATION="$(direnv_layout_dir)"
          mkdir -p "$NP_LOCATION"
          ADDITIONAL_CONFIG_FILE="$NP_LOCATION/nix-additional.conf"
          rm $ADDITIONAL_CONFIG_FILE
        fi
        if command -v git 2>&1 >/dev/null; then
          export NP_GIT="$(which git)"
        fi
        echo "max-jobs = auto" >> "$ADDITIONAL_CONFIG_FILE"
        echo "cores = 0" >> "$ADDITIONAL_CONFIG_FILE"
        export NP_CONF_ADDITIONAL_CONFIG="$ADDITIONAL_CONFIG_FILE"
        export NP_CONF_ADDITIONAL_FEATURES="ca-derivations dynamic-derivations recursive-nix"
      '';
    };
  };
}
