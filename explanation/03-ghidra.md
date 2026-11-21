# 03 – Décompiler avec Ghidra et s'y retrouver

## 1. Décompiler en masse (mode sans interface)

Plutôt que de cliquer fonction par fonction, on fait décompiler **tout** un binaire dans un seul
fichier `.c`, ensuite exploré avec `grep`, `awk`, `sed`. Script fourni : `tools/re/DecompileAll.java`
(post-script Ghidra : décompile chaque fonction, préfixée par `// ==== NOM @ adresse`).

```sh
mkdir -p re_work/decomp re_work/ghidra_proj

# la DLL du protocole (x64)
/opt/ghidra/support/analyzeHeadless re_work/ghidra_proj hsm_api \
    -import re_work/hsm_files/hsm_api_core_x64.dll \
    -scriptPath tools/re -postScript DecompileAll.java "$PWD/re_work/decomp/hsm_api_core_x64.c" \
    -deleteProject > re_work/decomp/hsm_api_core.log 2>&1
wc -l re_work/decomp/hsm_api_core_x64.c          # ~45 000 lignes

# le driver noyau Windows
/opt/ghidra/support/analyzeHeadless re_work/ghidra_proj sys \
    -import re_work/hsm_files/amd64/HercStreamMixDevice.sys \
    -scriptPath tools/re -postScript DecompileAll.java "$PWD/re_work/decomp/HercStreamMixDevice_sys.c" \
    -deleteProject > re_work/decomp/sys.log 2>&1

# l'application (gros binaire Qt : plus de mémoire)
GHIDRA_MAXMEM=12G /opt/ghidra/support/analyzeHeadless re_work/ghidra_proj app \
    -import re_work/app/Stream-control.exe \
    -scriptPath tools/re -postScript DecompileAll.java "$PWD/re_work/decomp/Stream-control.c" \
    -deleteProject > re_work/decomp/app.log 2>&1
```

## 2. Le firmware (binaire brut ARM)

Le `.hsm` = en-tête de 128 octets + image flash. On retire l'en-tête, puis on dit à Ghidra
**quel processeur** et **à quelle adresse** charger (la flash STM32 commence à `0x08000000`) :

```sh
mkdir -p re_work/fw
tail -c +129 re_work/hsm_files/HSM01_S32L4R7_v1_42.hsm > re_work/fw/hsm01_v1_42_body.bin
#   (équivalent : dd if=... of=... bs=128 skip=1)

/opt/ghidra/support/analyzeHeadless re_work/ghidra_proj fw \
    -import re_work/fw/hsm01_v1_42_body.bin \
    -processor ARM:LE:32:Cortex -loader BinaryLoader -loader-baseAddr 0x08000000 \
    -scriptPath tools/re -preScript FwSetup.java -postScript DecompileAll.java \
    "$PWD/re_work/decomp/fw_hsm01_v1_42.c" -deleteProject > re_work/decomp/fw.log 2>&1
grep -c "^// ====" re_work/decomp/fw_hsm01_v1_42.c      # nombre de fonctions trouvées
```

Sans aide, Ghidra trouve peu de fonctions dans une image brute. `FwSetup.java` (pre-script) :
lit la **table des vecteurs** (128 premières entrées à `0x08000000`), crée une fonction à chaque
gestionnaire d'interruption et active l'« ARM Aggressive Instruction Finder ». Résultat : nettement
plus de fonctions retrouvées (la première passe sans pre-script n'en trouvait que ~370), dont le
dispatcher du protocole qui manquait.

## 3. Méthodes pour trouver ce qui compte

**a) Par les constantes caractéristiques.** Le marqueur de trame `"SM"` = `0x4d53`, `"HERCULES"`…

```sh
cd re_work/decomp
grep -n -E "0x4d53|0x534d" hsm_api_core_x64.c | head
grep -n -E "case 0x(3[0-9a-f]|4[01]|1[0-7]|2[0-2]|50):" fw_hsm01_v1_42.c | head -40   # switch d'opcodes
```

**b) Lister les fonctions et naviguer par numéro de ligne.**

```sh
grep -n "^// ==== " hsm_api_core_x64.c | awk -F: '$1>2400 && $1<20000' | head -80
s=$(grep -n "^// ==== FUN_0800e128 " fw_hsm01_v1_42.c | cut -d: -f1); sed -n "$s,$((s+90))p" fw_hsm01_v1_42.c
# dans quelle fonction se trouve la ligne N ?
awk 'NR<=5979 && /^\/\/ ==== /{l=NR": "$0} NR==5979{print l}' fw_hsm01_v1_42.c
cd ../..
```

**c) Lire les données que le code référence.** Ghidra affiche `DAT_0800e374` : c'est une adresse
dans la flash (« literal pool ») qui contient la vraie valeur. Scripts :

```sh
python3 -I tools/re/fw_word.py re_work/fw/hsm01_v1_40_body.bin 0x0800e360 0x0800e368   # mots 32 bits
python3 -I tools/re/fw_word.py re_work/fw/hsm01_v1_42_body.bin --f64 0x0800e374        # double (ex. 204.0)
python3 -I tools/re/fw_find_word.py re_work/fw/hsm01_v1_40_body.bin 0x48000400         # qui charge GPIOB ?
python3 -I tools/re/fw_xref_range.py re_work/fw/hsm01_v1_40_body.bin \
    re_work/decomp/fw_hsm01_v1_40.c 0x48000400 0x48000401                               # → quelles fonctions
python3 -I tools/re/pe_read.py re_work/hsm_files/hsm_api_core_x64.dll 0x180108db0 16 '<2d'  # idem dans un PE
```

**d) Reconnaître les périphériques du microcontrôleur.** Les registres STM32 ont des adresses fixes
(RCC `0x40021000`, GPIOA `0x48000000`, LTDC `0x40016800`, TIM2 `0x40000000`…). Le script
`fw_peripherals.py` liste les adresses de périphériques présentes dans le firmware :

```sh
python3 -I tools/re/fw_peripherals.py re_work/fw/hsm01_v1_40_body.bin
python3 -I tools/re/fw_strings.py re_work/fw/hsm01_v1_40_body.bin    # chaînes lisibles (filtre le bruit graphique)
```

Les fonctions HAL de ST se reconnaissent ensuite à leur forme : `MX_LTDC_Init` remplit une structure
(timings, polarités) puis appelle l'init ; `HAL_GPIO_Init(port, &{Pin, Mode, Pull, Speed, Alternate})` ;
`HAL_TIM_PWM_ConfigChannel(htim, &cfg, canal)` etc. **L'ordre des champs des structures HAL** (lisible
dans les sources STM32Cube sur GitHub) permet de décoder les valeurs `local_1c = 0xffff; local_18 = 2…`.

**e) Pas de constante ? Chercher l'instruction.** Une adresse comme `0x40000000` (TIM2) tient dans une
instruction ARM (`mov.w`) et n'est pas dans un literal pool : on la cherche directement dans le C
décompilé (`grep -n "0x40000000"`), ou dans le désassemblage.

## 4. Utiliser l'interface graphique

Pour une fonction précise : `ghidra` → nouveau projet → *Import* (pour le firmware : *Language*
`ARM:LE:32:Cortex`, *Options* → *Base Address* `0x08000000`) → double-clic → *Analyze*. Puis
*Search → For Strings*, *Search → Memory*, clic droit → *References → Show References to*
(les « xrefs ») et renommer les fonctions au fur et à mesure (`L`). Le mode headless + `grep` sert à
débroussailler ; l'interface sert à comprendre une fonction en détail.

## 5. Ce que Ghidra a apporté (résumé)

- DLL : construction des trames, **table CRC16** (`FUN_180016490`), boucle CRC (`FUN_180016570`),
  analyse des rapports entrants (`FUN_180016740`), toutes les fonctions `HSM_*` (`docs/re/dll-map.md`).
- Driver Windows : la liste des IOCTL et surtout la requête **`RebootToDfuMode`** (0xF2 → bootloader).
- Firmware : dispatcher des commandes (`FUN_080099d0`), requêtes vendor (`FUN_08008d00`), horloges,
  écran (LTDC), PWM, ADC, algorithme des molettes (`FUN_0800e128`), fonctions cachées.
