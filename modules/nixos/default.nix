{
  perSystem = {
    nixosTests.nix-build-hello = import ./tests/nix-build-hello.nix;
    nixosTests.nix-build-header = import ./tests/nix-build-header.nix;
    nixosTests.nix-build-multi-source = import ./tests/nix-build-multi-source.nix;
    nixosTests.nix-build-shared-lib = import ./tests/nix-build-shared-lib.nix;
  };
}
