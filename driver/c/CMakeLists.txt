cmake_minimum_required(VERSION 3.16)
project(hsm01 VERSION 0.1.0 DESCRIPTION "Hercules Stream 100 userspace driver" LANGUAGES C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(HSM01_WERROR "Treat warnings as errors" OFF)
option(HSM01_BUILD_USB "Build the libusb transport and the CLI" ON)
option(HSM01_REQUIRE_IMAGE_LIBS "Fail when libpng, libjpeg or giflib is missing (release builds)" OFF)

add_compile_options(-Wall -Wextra -Wpedantic -Wshadow -Wconversion -Wno-sign-conversion)
if(HSM01_WERROR)
  add_compile_options(-Werror)
endif()

include(GNUInstallDirs)

# Pure protocol library (no libusb).
add_library(hsm01_proto STATIC src/proto.c src/free.c src/background.c)
target_include_directories(hsm01_proto PUBLIC
  $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
  $<INSTALL_INTERFACE:${CMAKE_INSTALL_INCLUDEDIR}>)
set_target_properties(hsm01_proto PROPERTIES POSITION_INDEPENDENT_CODE ON)

enable_testing()
add_executable(test_proto tests/test_proto.c)
target_link_libraries(test_proto PRIVATE hsm01_proto)
add_test(NAME proto COMMAND test_proto)
add_executable(test_free tests/test_free.c)
target_link_libraries(test_free PRIVATE hsm01_proto m)
add_test(NAME free COMMAND test_free)
add_executable(test_background tests/test_background.c)
target_link_libraries(test_background PRIVATE hsm01_proto)
add_test(NAME background COMMAND test_background)

install(TARGETS hsm01_proto ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR})

# Pictures for the CLI (backgrounds, boot logo, custom-firmware images): PNG / JPEG / GIF with the
# system libraries when present, BMP and PPM always. Not installed (CLI only).
add_library(hsm01_image STATIC tools/image.c)
target_link_libraries(hsm01_image PUBLIC hsm01_proto m)
add_executable(test_image tests/test_image.c)
target_include_directories(test_image PRIVATE tools)
target_link_libraries(test_image PRIVATE hsm01_image)
foreach(lib PNG JPEG GIF)
  find_package(${lib} QUIET)
  if(${lib}_FOUND)
    target_link_libraries(hsm01_image PRIVATE ${lib}::${lib})
    target_compile_definitions(hsm01_image PRIVATE HSM01_HAVE_${lib})
    target_link_libraries(test_image PRIVATE ${lib}::${lib})
    target_compile_definitions(test_image PRIVATE HSM01_HAVE_${lib})
  elseif(HSM01_REQUIRE_IMAGE_LIBS)
    message(FATAL_ERROR "${lib} development files not found (HSM01_REQUIRE_IMAGE_LIBS)")
  else()
    message(STATUS "${lib} not found: the CLI will not read ${lib} pictures")
  endif()
endforeach()
add_test(NAME image COMMAND test_image)
install(FILES include/hsm01/proto.h include/hsm01/free.h include/hsm01/background.h DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}/hsm01)

if(HSM01_BUILD_USB)
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(LIBUSB REQUIRED IMPORTED_TARGET libusb-1.0)
  find_package(Threads REQUIRED)

  add_library(hsm01 STATIC src/usb.c src/free_usb.c)
  target_link_libraries(hsm01 PUBLIC hsm01_proto PkgConfig::LIBUSB Threads::Threads)

  add_executable(hsm01-c tools/hsm01_cli.c tools/hsm01_fw.c)
  target_link_libraries(hsm01-c PRIVATE hsm01 hsm01_image m)

  install(TARGETS hsm01 ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR})
  install(TARGETS hsm01-c RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
  install(FILES include/hsm01/usb.h include/hsm01/free_usb.h DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}/hsm01)
endif()
