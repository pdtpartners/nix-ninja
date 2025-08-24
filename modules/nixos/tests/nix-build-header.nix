{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-header";
  inputsFrom = [ pkgs.example-header ];
  expectedStdout = "Hello dynamic derivations!";
} args
