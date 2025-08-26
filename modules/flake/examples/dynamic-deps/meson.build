project('example-dynamic-deps', 'cpp',
  version : '0.1.0')

# Custom target to generate the C++ file that depends on config.h
generated_cpp = custom_target('generate_cpp',
  output : 'generated.cpp',
  input : 'generate.sh',
  command : [find_program('bash'), '@INPUT@'],
  build_by_default : true
)

sources = [
  'main.cpp',
  generated_cpp,
]

includes = include_directories('.')

executable('main', 
  sources,
  include_directories : includes)
