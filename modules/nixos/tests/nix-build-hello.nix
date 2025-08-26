{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-hello";
  inputsFrom = [ pkgs.example-hello ];
  cmdline = "hello";
  expectedStdout = "Hello dynamic derivations!";
} args
