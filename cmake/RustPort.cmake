# The Rust port of libgit2's core (rust/libgit2-rs).
#
# With USE_RUST=ON, the C sources listed in rust/ported_sources.txt leave the
# build, and the Rust static library that replaces them is linked into
# libgit2, its tests and the CLI.  The port is written and tested for Linux
# x86-64 with the configuration this file checks; on any other platform or
# configuration the C sources are built as before.

if(UNIX AND NOT APPLE AND CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64")
	set(USE_RUST_DEFAULT ON)
else()
	set(USE_RUST_DEFAULT OFF)
endif()
option(USE_RUST "Build libgit2's core from the Rust port in rust/" ${USE_RUST_DEFAULT})
set(RUST_SANITIZER "" CACHE STRING "Build the Rust port with -Zsanitizer=<value> (nightly): address, thread or empty")
set(CARGO_COMMAND "cargo" CACHE STRING "The cargo executable used to build the Rust port")

set(RUST_PORTED_SOURCES "")

if(USE_RUST)
	if(WIN32)
		message(FATAL_ERROR "USE_RUST is supported on Unix only; the win32 branches stay in C")
	endif()

	set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${PROJECT_SOURCE_DIR}/rust/ported_sources.txt")
	file(STRINGS "${PROJECT_SOURCE_DIR}/rust/ported_sources.txt" _ported REGEX "^[^#]")
	foreach(_f ${_ported})
		string(STRIP "${_f}" _f)
		if(NOT _f STREQUAL "")
			list(APPEND RUST_PORTED_SOURCES "${PROJECT_SOURCE_DIR}/${_f}")
		endif()
	endforeach()

	set(RUST_DIR "${PROJECT_SOURCE_DIR}/rust")
	set(RUST_TARGET_DIR "${PROJECT_BINARY_DIR}/rust-target")
	set(RUST_ENV "CARGO_TARGET_DIR=${RUST_TARGET_DIR}" "LIBGIT2_BUILD_DIR=${PROJECT_BINARY_DIR}")
	set(RUST_ARGS build --release --manifest-path "${RUST_DIR}/Cargo.toml" -p libgit2-rs --lib)

	if(RUST_SANITIZER)
		list(APPEND RUST_ENV "RUSTFLAGS=-Zsanitizer=${RUST_SANITIZER} -Cforce-frame-pointers=yes")
		list(APPEND RUST_ARGS -Zbuild-std --target x86_64-unknown-linux-gnu)
		set(RUST_STATICLIB "${RUST_TARGET_DIR}/x86_64-unknown-linux-gnu/release/liblibgit2_rs.a")
		set(_cargo "${CARGO_COMMAND}" +nightly)
	else()
		set(RUST_STATICLIB "${RUST_TARGET_DIR}/release/liblibgit2_rs.a")
		set(_cargo "${CARGO_COMMAND}")
	endif()

	add_custom_target(libgit2_rust ALL
		COMMAND ${CMAKE_COMMAND} -E env ${RUST_ENV} ${_cargo} ${RUST_ARGS}
		WORKING_DIRECTORY "${RUST_DIR}"
		BYPRODUCTS "${RUST_STATICLIB}"
		USES_TERMINAL
		COMMENT "Building the Rust port (rust/libgit2-rs)")

	add_library(git2_rust STATIC IMPORTED GLOBAL)
	set_target_properties(git2_rust PROPERTIES IMPORTED_LOCATION "${RUST_STATICLIB}")
	add_dependencies(git2_rust libgit2_rust)

	# Rust's standard library needs these on glibc.
	list(APPEND LIBGIT2_SYSTEM_LIBS git2_rust dl m)
	list(APPEND LIBGIT2_PC_LIBS "-ldl" "-lm")
	# GIT_RUST in git2_features.h: headers whose types the Rust defines.
	set(GIT_RUST 1)
	add_feature_info("Rust port" ON "core ported to Rust (rust/ported_sources.txt)")
else()
	add_feature_info("Rust port" OFF "core built from C")
endif()
