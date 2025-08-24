{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-incremental";
  inputsFrom = [ pkgs.example-incremental ];
  expectedStdout = "Hello dynamic derivations!";
} args
