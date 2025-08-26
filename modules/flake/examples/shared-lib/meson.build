project('example-sharedlib', 'c',
  version : '0.1.0')

# Build the first shared library
word1lib = shared_library('word1lib',
  sources : ['word1lib.c'],
  install : true)

# Build the second shared library
word2lib = shared_library('word2lib',
  sources : ['word2lib.c'],
  install : true)

# Build the executable that links against both shared libraries
executable('main',
  sources : ['main.c'],
  link_with : [word1lib, word2lib],
  install : true)
