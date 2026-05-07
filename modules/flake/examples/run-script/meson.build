project('example-run-script', 'cpp',
  version : '0.1.0')

# Execute the script directly (unlike the dynamic-deps example, which goes
# through `bash`): the store copy of gen.sh must keep its executable bit.
generated_cpp = custom_target('generate_cpp',
  output : 'generated.cpp',
  command : [files('gen.sh')],
  build_by_default : true
)

executable('main', ['main.cpp', generated_cpp])
