# Ladybird's configuration looks for Qt whenever it builds its GUI targets, for its Qt browser. zgx builds the engine
# and its own host instead (see zgx-project-include.cmake), so nothing uses or links Qt: this stands in for it, and
# defines no targets, so anything that did use it would fail to configure.
set(Qt6_FOUND TRUE)
set(Qt6_VERSION 6.10.0)
foreach (component IN LISTS Qt6_FIND_COMPONENTS)
    set(Qt6${component}_FOUND TRUE)
    set(Qt6_${component}_FOUND TRUE)
endforeach()
