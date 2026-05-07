#!/bin/sh

# This script is executed *directly* by the ninja rule (no interpreter in
# the command line), so building this example verifies that nix-ninja
# preserves the executable bit when uploading input files to the store.
# The shebang is /bin/sh because that is the one interpreter path present
# inside the Nix build sandbox.
cat > generated.cpp << 'EOF'
const char* get_message() {
    return "Hello run-script example!";
}
EOF
