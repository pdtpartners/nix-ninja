{ self, ... }:
{
  perSystem = { pkgs, system, ... }: {
    packages = {
      inherit (pkgs)
        nix-ninja
        nix-ninja-task
        nix-ninja-llvm-coverage
      ;

      default = pkgs.buildEnv {
        name = "nix-ninja";
        paths = with pkgs; [
          nix-ninja
          nix-ninja-task
        ];
      };
    };

    legacyPackages = {
      example-hello = pkgs.example-hello.target;
      example-header = pkgs.example-header.target;
      example-multi-source = pkgs.example-multi-source.target;
      example-shared-lib = pkgs.example-shared-lib.target;
      example-nix = pkgs.example-nix.target;
    };

    devShells.default = pkgs.craneLib.devShell {
      checks = self.checks.${system};

      packages = with pkgs; [
        agg
        gnumake
        just
        meson
        taplo
      ];
    };
  };
}
