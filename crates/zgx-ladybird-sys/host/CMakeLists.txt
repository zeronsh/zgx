# zgx's Ladybird host: the process that runs Ladybird's LibWebView for a zgx app (see ../README.md). Built inside
# Ladybird's own build, in the place of its Qt browser (../ladybird/zgx-project-include.cmake), as the target `ladybird`
# so that Ladybird's checks and helper-process rules apply to it.

add_executable(ladybird
    main.cpp
)
set_target_properties(ladybird PROPERTIES OUTPUT_NAME zgx-ladybird-host)

target_link_libraries(ladybird PRIVATE AK LibCore LibGfx LibIPC LibMain LibThreading LibWebView LibURL LibWakeLock LibGamepad hubipc)
target_link_libraries(ladybird PRIVATE OpenSSL::Crypto OpenSSL::SSL)
if (LINUX OR APPLE)
    target_compile_definitions(ladybird PRIVATE LADYBIRD_HUB_IN_OWN_PROCESS)
endif()
target_include_directories(ladybird PRIVATE ${LADYBIRD_SOURCE_DIR} ${LADYBIRD_SOURCE_DIR}/Services ${CMAKE_CURRENT_BINARY_DIR})

# Helpers sit where LibWebView looks for them from the host: beside it on macOS, in libexec elsewhere.
foreach (helper IN LISTS ladybird_helper_processes)
    if (APPLE)
        set_target_properties(${helper} PROPERTIES RUNTIME_OUTPUT_DIRECTORY "$<TARGET_FILE_DIR:ladybird>")
    else()
        set_target_properties(${helper} PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/${IN_BUILD_PREFIX}${CMAKE_INSTALL_LIBEXECDIR}")
    endif()
endforeach()
add_dependencies(ladybird ${ladybird_helper_processes})

# Ladybird's resources (themes, fonts, its internal pages), where LibWebView looks from the host: <prefix>/share/Lagom
# beside <prefix>/bin, or the bundle's Resources on macOS.
include(${LADYBIRD_SOURCE_DIR}/UI/cmake/ResourceFiles.cmake)
if (APPLE)
    copy_resources_to_build("$<TARGET_FILE_DIR:ladybird>/../Resources" ladybird)
else()
    copy_resources_to_build("${CMAKE_BINARY_DIR}/${IN_BUILD_PREFIX}${CMAKE_INSTALL_DATADIR}/Lagom" ladybird)
endif()
add_dependencies(ladybird ladybird_build_resource_files)

# The install zgx finds the host in (ladybird/build.py installs component zgx_ladybird): on Linux bin/, the helpers in
# libexec/, libraries in lib/ and resources in share/Lagom; on macOS the host and helpers in MacOS/ beside Resources/
# and lib/, as LibWebView looks from the host. LICENSES.txt is Ladybird's list of the notices of what it is built from.
include(GNUInstallDirs)
include(install_vcpkg_libraries)
if (APPLE)
    set(zgx_host_dir MacOS)
    set(zgx_helper_dir MacOS)
    set(zgx_resource_dir Resources)
else()
    set(zgx_host_dir ${CMAKE_INSTALL_BINDIR})
    set(zgx_helper_dir ${CMAKE_INSTALL_LIBEXECDIR})
    set(zgx_resource_dir ${CMAKE_INSTALL_DATADIR}/Lagom)
endif()
set(zgx_vcpkg_dependencies "")
if (LADYBIRD_INSTALLS_VCPKG_LIBRARIES)
    set(zgx_vcpkg_dependencies RUNTIME_DEPENDENCY_SET zgx_ladybird_vcpkg_libraries)
endif()
install(TARGETS ladybird ${zgx_vcpkg_dependencies} RUNTIME DESTINATION ${zgx_host_dir} COMPONENT zgx_ladybird)
install(TARGETS ${ladybird_helper_processes} ${zgx_vcpkg_dependencies} RUNTIME DESTINATION ${zgx_helper_dir} COMPONENT zgx_ladybird)

include("${LADYBIRD_SOURCE_DIR}/Meta/CMake/get_linked_lagom_libraries.cmake")
set(zgx_lagom_libraries "")
foreach (application IN ITEMS ladybird ${ladybird_helper_processes})
    get_linked_lagom_libraries("${application}" "${application}_lagom_libraries")
    list(APPEND zgx_lagom_libraries ${${application}_lagom_libraries})
endforeach()
list(REMOVE_DUPLICATES zgx_lagom_libraries)
list(REMOVE_ITEM zgx_lagom_libraries ladybird)
install(TARGETS ${zgx_lagom_libraries} ${zgx_vcpkg_dependencies}
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR} COMPONENT zgx_ladybird NAMELINK_SKIP
    ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR} COMPONENT zgx_ladybird_development EXCLUDE_FROM_ALL)
if (zgx_vcpkg_dependencies)
    ladybird_install_vcpkg_libraries(zgx_ladybird_vcpkg_libraries DESTINATION ${CMAKE_INSTALL_LIBDIR} COMPONENT zgx_ladybird)
endif()

if (APPLE)
    install(DIRECTORY "$<TARGET_FILE_DIR:ladybird>/../Resources/" DESTINATION ${zgx_resource_dir} COMPONENT zgx_ladybird)
else()
    install(DIRECTORY "${CMAKE_BINARY_DIR}/${IN_BUILD_PREFIX}${CMAKE_INSTALL_DATADIR}/Lagom/" DESTINATION ${zgx_resource_dir} COMPONENT zgx_ladybird)
endif()
install(FILES "${CMAKE_BINARY_DIR}/LICENSES.txt" DESTINATION . COMPONENT zgx_ladybird)
