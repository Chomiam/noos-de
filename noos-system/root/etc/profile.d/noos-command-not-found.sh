# NOOS : « commande introuvable » propose le paquet qui la fournit (bash, shells de connexion).
if [ -n "${BASH_VERSION:-}" ] && [ -x /opt/noos/bin/noos-pkg ] && ! declare -F command_not_found_handle >/dev/null; then
    command_not_found_handle() {
        /opt/noos/bin/noos-pkg provides --hint -- "$1"
        return 127
    }
fi
