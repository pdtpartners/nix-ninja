{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-header";
  inputsFrom = [ pkgs.example-header ];
  cmdline = "hello";
  expectedStdout = "Hello header example!";
} args
