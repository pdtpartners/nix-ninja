#!/usr/bin/env bash

# Script to generate a C++ source file that depends on config.h
cat > generated.cpp << 'EOF'
#include <stdio.h>
#include <nlohmann/json.hpp>
#include "config.h"

const char* get_example_name() {
    return EXAMPLE_NAME;
}
EOF

echo "Generated generated.cpp with dependency on config.h"
