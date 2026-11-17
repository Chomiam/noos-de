Quatrième préversion de NOOS, basée sur **Debian sid**.

**Installation réseau** : l'ISO démarre un système live complet (toutes les dépendances de NOOS y sont) ; l'installateur télécharge ensuite Debian sid (debootstrap) et le paquet `noos` depuis le dépôt APT du projet. Une connexion Internet est nécessaire pendant l'installation.

### Nouveautés depuis l'alpha 3
- **Énergie dans la barre des tâches** : sur portable, batterie dessinée selon le niveau et l'état (en charge, branchée, faible, critique) avec l'autonomie restante au survol ; sur PC fixe, jauge du mode d'alimentation. Au clic : pourcentage, santé de la batterie, consommation, modes Économie / Équilibré / Performances. Molette : mode voisin. Alerte à 10 %.
- **Portables** : l'installateur détecte un portable et installe **auto-cpufreq** (économie d'énergie automatique), piloté depuis le menu Énergie et Paramètres › Alimentation.
- **Démarrage** : correction de l'erreur de montage d'efivarfs (busybox dans l'initramfs), firmwares Intel graphiques et Wi-Fi (l'ISO live les embarque pour installer en Wi-Fi), rtkit, upower, téléphones et appareils MTP.

### Depuis l'alpha 2 (rappel)
- **Debian sid** : apt, dépôt APT signé (https://chomiam.github.io/noos-de/), GRUB signé (Secure Boot), architecture i386 activée (Steam, Wine).
- **Applis X11** par Xwayland, avec un gestionnaire de fenêtres X11 intégré au compositeur.
- **Écran** : sortie HDR10 (PQ, BT.2020, 10 bits) ; VRR avec la cause précise quand il est indisponible.
- **Pare-feu applicatif NOOS** (nftables + NFQUEUE) avec fenêtre d'autorisation par programme.
- **Bluetooth** (ajout, appairage) et **Imprimantes et scanners** (CUPS, SANE) dans Paramètres.
- **Partage de fichiers** en réseau local : Samba, SSH / SFTP, WebDAV ; page **Réseau** dans l'Explorateur.
- **Logiciels** : AppImage (installées dans ~/Applications, raccourci dans le menu Démarrer) et paquets `.deb`.
- **Explorateur** : clic droit sur l'élément visé, cadre de sélection en direct.
- Bureaux virtuels, plusieurs écrans, mode édition du bureau (barre sur les 4 bords, widgets).
- Disques, Archives, éditeur de texte, Photos, écran de verrouillage animé, mises à jour en arrière-plan.
- Arrière-plan : pages suivantes pour Wallhaven et Bing.
- Applis dans la langue du système (Chrome, Electron), écran de connexion sur la console 1.

### Vérifier l'image
```
sha256sum -c SHA256SUMS
```

⚠️ Préversion : à tester de préférence en machine virtuelle ou sur un PC sans données importantes (le disque choisi est entièrement effacé).
