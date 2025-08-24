{ self, ... }:
{
  perSystem = { pkgs, system, ... }: {
    packages = {
      inherit (pkgs)
        nix-ninja
        nix-ninja-task
        nix-ninja-llvm-coverage
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
        taplo
      ];
    };
  };
}
