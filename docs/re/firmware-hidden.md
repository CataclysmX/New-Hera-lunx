# Fonctions cachées / non documentées du firmware (v1.42, idem 1.40 sauf mention)

| Fonction | Déclenchement | Effet | Preuve |
|----------|---------------|-------|--------|
| **Overlay de diagnostic** | maintenir **clic molette 1**, double-appui rapide (< 500 ms) sur **bouton 4** | flag RAM 0x2000011d : barre au bord droit + texte `FPS: %d (max %dms) FC=%dk +%d.%.2d:%.2d:%.2d, USB: %d buf./sec (Max.Q:%d/%d), E=%d` (FPS, rendu max, trames, uptime, débit USB, file ISO max /33, erreurs) | `FUN_0800e610` (combo), `FUN_0801075c` (rendu) |
| **Mode autonome** | **power + bouton 1** (bascule) ; automatique si l'hôte se tait ~1 900 tours de rendu | entrées plus remontées, molettes ignorées, état VU sauvegardé/restauré au retour hôte ; luminosité 100 % / 80 % | flag 0x2000016c, `FUN_0800e610`, `FUN_0800e128`, `FUN_0801075c` |
| Luminosité locale | power court / long | +20 % cyclique / écran off | `FUN_0800e610` |
| **Reboot DFU** | requête vendor `0x40 0xF2 wValue=0 wIndex=0xE053` (testé ✔) ; clic molettes 1+4 au branchement (forum) | mot magique RAM + reset → `FUN_08006254` saute au bootloader ROM (0x1FFF0000) | `FUN_08008d00`, `FUN_08006254`, `FUN_08005e94` |
| Mode usine | SM `20 01` | autorise `0x21` (n° série, banque 0) et `0x22` (code usine) → mémoire info | `FUN_080099d0` |
| Requêtes vendor `0x0e` / `0x0f` | non utilisées par la DLL | `0x0f` règle 2 × u16 (fenêtre ?), `0x0e` lit l'espace libre du ring ISO + appelle `FUN_08000a50` | `FUN_08008d00` |
| Opcodes no-op | SM `0x15`, `0x16` | rien (réservés) | `FUN_080099d0` |
| Splash | SM `50 80` / `50 40` | écrit le fond courant comme logo de boot / restaure le logo d'usine | `FUN_080099d0`, `FUN_0800da6c` |

Chaînes : `" FLASH CONFIG"`, `" EXTFLASH INT"` (probables descripteurs USB d'un mode non utilisé),
`"HSMSetup"` (référence non trouvée). Le reste du binaire = images RLE embarquées (icônes, polices).
À explorer : endpoint bulk OUT 0x02 (déclaré mais jamais utilisé par la DLL ; le driver Windows
mentionne "IOCTL Bulk Update" / "EEPROM rewrite").
