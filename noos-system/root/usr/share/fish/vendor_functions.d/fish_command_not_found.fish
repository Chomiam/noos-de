# NOOS : « commande introuvable » propose le paquet qui la fournit (noos-pkg provides).
function fish_command_not_found
    set -l pkg /opt/noos/bin/noos-pkg
    if test -x $pkg
        $pkg provides --hint -- $argv[1]
    else
        __fish_default_command_not_found_handler $argv
    end
end
