# 05 – Reconstruire le protocole officiel et le tester en vrai

Résultat complet : `docs/protocol/` (`usb.md`, `framing.md`, `commands.md`, `inputs.md`).
Ici : **comment** on y est arrivé.

## 1. La trame, guidée par la DLL

Dans `hsm_api_core_x64.c`, la fonction qui écrit `'S' 'M'` (constante `0x4d53`) construit le message :
`[SM][longueur u16][CRC16 u16][n° séquence u16][commandes…][0x00]`. Deux fonctions voisines génèrent
une **table de CRC** (`FUN_180016490`) et la parcourent (`FUN_180016570`).

On réécrit ça en Python (`tools/re/hsm01_proto.py`) et on vérifie sur **toutes** les trames capturées :

```sh
for f in re_work/captures/*.tsv; do echo "$f: $(python3 -I tools/re/hsm01_proto.py check $f | tail -3)"; done
# → 3305/3305 CRC corrects (sorties), 6483/6483 checksums corrects (entrées)
```

Quand un CRC ne colle pas, on extrait des **vecteurs de test** (messages courts) pour tester des
variantes d'algorithme (polynôme, init, sens) :

```sh
python3 -I tools/re/test_vectors.py re_work/captures/wireshark.tsv 40 1
python3 -I tools/re/test_vectors.py re_work/captures/wireshark_debug.tsv 400 1
```

Les constantes que le code charge depuis la mémoire se lisent directement dans le fichier :

```sh
python3 -I tools/re/pe_read.py re_work/hsm_files/hsm_api_core_x64.dll 0x180108e00 24 '<6I'
```

## 2. Les commandes (opcodes)

Le firmware contient un `switch` sur l'opcode (dispatcher `FUN_080099d0`) : il donne la **liste
exhaustive** des commandes et leur **longueur**. On décode ensuite les captures avec cette table :

```sh
python3 -I tools/re/hsm01_proto.py decode re_work/captures/wireshark.tsv      # statistiques par opcode
python3 -I tools/re/hsm01_proto.py decode-v re_work/captures/wireshark.tsv | head   # détail par trame
python3 -I tools/re/hsm01_proto.py decode-v re_work/captures/testzireshqrk3.tsv | tr ' ' '\n' \
  | grep -E "^(10|12|14|30|31|32|34|38|40|41):" | sort | uniq -c | sort -k2 | head -70
```

Le sens de chaque opcode vient du croisement : nom de la fonction `HSM_Set…` de la DLL qui l'émet +
ce que le firmware en fait (ex. `0x30` = LED, `0x31` = luminosité, `0x35` icône, `0x41` niveau VU).

## 3. Les entrées (molettes, boutons)

Rapport interrupt de 64 octets : `[longueur][boutons u16][4 × i16 molettes][état écran][message sm]`.
Analyse : DLL `FUN_180016740` + `in_reports.py` (fichier 04).

## 4. Tester sur l'appareil (Linux, pyusb)

```sh
python3 -I tools/hsm01_live.py 6                    # init + keep-alive + affichage des entrées 6 s
python3 -I tools/hsm01_live.py 6 --leds=1210        # + LEDs (0 éteint, 1 allumé, 2 clignote)
python3 -I tools/hsm01_live.py 4 --leds=1210 --raw  # rapports bruts
python3 -I tools/hsm01_replay.py re_work/captures/wireshark.tsv --hold=15   # rejoue la session officielle
```

Le **rejeu** est le test décisif : les 600 trames du logiciel officiel, ré-encapsulées par notre code
(nouvelle séquence, nouveau CRC), sont acceptées sans erreur (`err=0`, `lastInv=0` dans le statut) et
l'écran affiche icônes, labels, VU-mètres → le protocole est compris.

Pour le mapping physique des boutons : lancer le moniteur et manipuler dans un ordre annoncé
(« molette 1→4, bouton 1→4, clic molette 1→4, power »), puis relever les bits qui changent.

## 5. Les requêtes de contrôle (vendor)

Vues dans la capture et dans le firmware (`FUN_08008d00`) :

| bmRequestType | bRequest | wValue | wIndex | Effet |
|---------------|----------|--------|--------|-------|
| `0xC0` | `0x02` | 0 | 0 | version firmware `[mineur][majeur]` |
| `0xC0` | `0x52` | offset | banque | mémoire info (n° de série en banque 0) |
| `0x40` | `0xF2` | **0** | **0xE053** | **redémarrage en bootloader DFU** (driver Windows `RebootToDfuMode`) |

La dernière ligne a d'abord échoué (« STALL ») avec `wValue`/`wIndex` inversés : c'est le driver
Windows décompilé qui a donné le bon ordre. Leçon : **toujours vérifier dans le code qui l'émet**.
