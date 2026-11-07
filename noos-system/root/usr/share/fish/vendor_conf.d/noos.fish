# NOOS : réglages de fish pour tous les comptes (fichier du paquet noos).
# Pour les changer, redéfinissez-les dans ~/.config/fish/config.fish (lu après ce fichier),
# ex. « set -g fish_greeting "Bonjour" » ; NOOS_NO_FASTFETCH=1 désactive fastfetch.

# Pas de message de bienvenue de fish : fastfetch le remplace.
set -g fish_greeting

# fastfetch à l'ouverture d'un terminal (shell interactif seulement, pas dans les scripts).
if status is-interactive; and not set -q NOOS_NO_FASTFETCH; and type -q fastfetch
    fastfetch
end
