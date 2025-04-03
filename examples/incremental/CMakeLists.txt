cmake_minimum_required(VERSION 3.10)
project(example)

add_library(util util.c util.h)
add_executable(main main.c)
target_link_libraries(main util)
