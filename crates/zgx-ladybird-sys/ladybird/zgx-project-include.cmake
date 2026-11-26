# Included by Ladybird's top-level project() (CMAKE_PROJECT_ladybird_INCLUDE), so that its unmodified source builds the
# engine, its helper processes and zgx's host, and not its Qt browser. Ladybird adds UI after Services; we add the host
# in its place, as the target `ladybird`, so Ladybird's own checks on that target (its helpers are found beside it, and
# it must not link Skia) apply to the host.

if (NOT DEFINED ZGX_LADYBIRD_HOST_DIR)
    message(FATAL_ERROR "ZGX_LADYBIRD_HOST_DIR names zgx's host source")
endif()

function(add_subdirectory source_dir)
    if ("${source_dir}" STREQUAL "UI" AND "${CMAKE_CURRENT_SOURCE_DIR}" STREQUAL "${ladybird_SOURCE_DIR}")
        _add_subdirectory("${ZGX_LADYBIRD_HOST_DIR}" "${CMAKE_BINARY_DIR}/zgx-host")
        return()
    endif()
    _add_subdirectory(${ARGV})
endfunction()
