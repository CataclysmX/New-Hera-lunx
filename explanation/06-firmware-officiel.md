# 06 – Le firmware officiel : récupération, versions, mises à jour, fonctions cachées

## 1. Le fichier `.hsm`

```sh
python3 -I tools/re/fwfg_header.py re_work/hsm_files/HSM01_S32L4R7_v1_42.hsm   # décode l'en-tête "FWFG"
```

En-tête de 128 octets (`FWFG`, "Hercules", "HSM01", "SL4R" = STM32L4R, version, date, sommes de
contrôle), puis l'image brute de la flash à partir de `0x08000000` : vecteur de pile et de reset en
tête, **non chiffrée** → directement analysable avec Ghidra (fichier 03).

Recherche de l'algorithme des sommes de contrôle (essais CRC32 classiques sur différentes plages) :

```sh
python3 -I tools/re/fw_crc_probe.py re_work/hsm_files/HSM01_S32L4R7_v1_42.hsm
```

(non concluant : l'algorithme est dans `GuiSTDFUDevUpdate64.dll` ; inutile pour flasher via DFU.)

## 2. Le serveur de mises à jour

L'URL a été trouvée dans les chaînes de l'application (fichier 02) :

```sh
mkdir -p re_work/net && cd re_work/net
curl -sS -m 30 -A "Mozilla/5.0" -o update.xml https://ts.hercules.com/download/streaming/update/update.xml
curl -sS -m 30 -A "Mozilla/5.0" -o beta_update.xml https://ts.hercules.com/download/streaming/updateBeta/update.xml
grep -E "number|downloadURL|firmware|hash|date|controller" beta_update.xml
curl -sS -m 120 -A "Mozilla/5.0" -o HSM01_S32L4R7_v1_40.hsm \
     https://ts.hercules.com/download/streaming/update/HSM01_S32L4R7_v1_40.hsm
md5sum HSM01_S32L4R7_v1_40.hsm ../hsm_files/HSM01_S32L4R7_v1_42.hsm   # comparer aux MD5 du manifeste
python3 -I ../../tools/re/bin_diff.py HSM01_S32L4R7_v1_40.hsm ../hsm_files/HSM01_S32L4R7_v1_42.hsm 6
cd ../..
tail -c +129 re_work/net/HSM01_S32L4R7_v1_40.hsm > re_work/fw/hsm01_v1_40_body.bin
/opt/ghidra/support/analyzeHeadless re_work/ghidra_proj fw140 -import re_work/fw/hsm01_v1_40_body.bin \
    -processor ARM:LE:32:Cortex -loader BinaryLoader -loader-baseAddr 0x08000000 \
    -scriptPath tools/re -preScript FwSetup.java -postScript DecompileAll.java \
    "$PWD/re_work/decomp/fw_hsm01_v1_40.c" -deleteProject > re_work/decomp/fw140.log 2>&1
```

On analyse de préférence la **1.40**, puisque c'est la version qui tourne sur l'appareil. Les
adresses de fonctions changent un peu d'une version à l'autre.

Constat : le serveur ne propose que la **1.40** ; la 1.42 n'existe que dans l'installeur. L'image 1.40
téléchargée sert aussi de **référence** pour vérifier la sauvegarde de l'appareil (fichier 07).

## 3. Lancer le logiciel Windows sous Linux ?

Testé et écarté : le logiciel passe par des **drivers noyau Windows** (KMDF) via `SetupDi` +
`DeviceIoControl` ; Wine/Proton ne charge pas de drivers noyau. Pour l'utiliser : une VM Windows avec
passage USB de `06f8:e053` **et** `0483:df11` (le mode mise à jour).

## 4. Comprendre le matériel à travers le firmware

C'est le firmware officiel décompilé qui a servi de **schéma électronique** (rien n'a été ouvert).
Adresses ci-dessous : `fw_hsm01_v1_40.c`, sauf les molettes (`fw_hsm01_v1_42.c`) :

| Question | Où regarder dans le firmware | Réponse |
|----------|------------------------------|---------|
| horloges | `SystemClock_Config` (`FUN_0800616c`) : structures `RCC_OscInit` / `PeriphClkInit` | MSI 4 MHz → PLL 120 MHz ; LTDC ÷8 = 10 MHz ; pas de quartz LSE |
| écran | `MX_LTDC_Init` (`FUN_0800d000`) + `HAL_LTDC_MspInit` (`FUN_08002500`) | 480×272 RGB565, timings, broches AF11, PA8 = DISP |
| rétroéclairage, LEDs | `MX_TIM2_Init`, `MX_TIM8_Init`, `FUN_08010a70` | TIM2 CH1 PA5 (+ PB10 enable), TIM8 CH1-4 PC6-9 |
| boutons | `MX_GPIO_Init` (`FUN_0800ce0c`), lecture `FUN_0800d5ec` | PB11-15, PC10-12, PA15 (actifs bas) |
| molettes | ADC + `FUN_0800e128` | 2 curseurs par molette, algorithme au fichier 11 |
| requêtes USB | `FUN_08008d00` | 0x02 version, 0x52 info, 0xF2 DFU |

Méthode pour une broche : chercher l'adresse du port GPIO (`fw_find_word.py 0x48000400` pour GPIOB),
lister les fonctions qui l'utilisent (`fw_xref_range.py`), puis lire le masque passé à `HAL_GPIO_Init`
(ex. `0x400` = broche 10). Synthèse : `docs/firmware/hardware.md`.

## 5. Fonctions cachées

En suivant tous les cas du dispatcher et des combinaisons de boutons dans la boucle principale :
overlay de debug, mode autonome, entrée DFU par **clics molettes 1+4 au branchement**, commandes usine.
Détails : `docs/re/firmware-hidden.md`. Commandes d'appoint :

```sh
python3 -I tools/re/fw_strings.py re_work/fw/hsm01_v1_42_body.bin | head -60
grep -n -E "== 0x52|case 0x52" re_work/decomp/fw_hsm01_v1_42.c | head
```
