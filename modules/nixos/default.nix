{
  perSystem = {
    nixosTests.nix-build-hello = import ./tests/nix-build-hello.nix;
    nixosTests.nix-build-header = import ./tests/nix-build-header.nix;
    nixosTests.nix-build-incremental = import ./tests/nix-build-incremental.nix;
  };
}
