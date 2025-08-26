{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-shared-lib";
  inputsFrom = [ pkgs.example-shared-lib ];
  cmdline = "main";
  expectedStdout = "Hello shared-lib example!";
} args
