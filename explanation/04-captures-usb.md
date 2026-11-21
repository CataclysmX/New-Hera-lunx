# 04 – Capturer le trafic USB du logiciel officiel

Le code décompilé dit *comment* les trames sont construites ; une capture donne des **exemples réels**
pour vérifier qu'on a bien compris (CRC, ordre des champs, cadence).

## 1. Capturer (Windows, là où tourne le logiciel officiel)

1. Installer **Wireshark** avec **USBPcap** (proposé par l'installeur de Wireshark).
2. Lancer Wireshark, choisir l'interface `USBPcap1` (celle du contrôleur où est branché le Stream 100).
3. Démarrer la capture **avant** de brancher l'appareil (pour avoir l'énumération : descripteurs,
   `SET_CONFIGURATION`, requêtes vendor), puis lancer Stream Control, changer des pages, tourner les
   molettes, appuyer sur les boutons.
4. Filtre d'affichage utile : `usb.idVendor == 0x06f8 || usb.device_address == N`.
5. Exporter : *Fichier → Exporter les dissections de paquets → En texte brut*, en cochant
   « octets des paquets ». On obtient `wireshark.txt`, `testzireshqrk3.txt`…

Sous Linux le même travail se fait avec `usbmon` (utile pour observer **nos** programmes) :

```sh
sudo modprobe usbmon
lsusb -d 06f8:e053          # "Bus 001 Device 012" → interface usbmon1
wireshark                    # interface usbmon1, filtre usb.device_address == 12
```

## 2. Transformer l'export texte en tableau

```sh
mkdir -p re_work/captures
for f in wireshark.txt wireshark_debug.txt testzireshqrk3.txt testZireshqrk.txt; do
  python3 -I tools/re/wsdump.py $f > re_work/captures/${f%.txt}.tsv
done
wc -l re_work/captures/*.tsv
# répartition des transferts par type et endpoint :
for f in re_work/captures/*.tsv; do echo "== $f"; cut -f3,4,5,6 $f | sort | uniq -c | sort -rn | head -12; done
```

Colonnes du `.tsv` : n° de trame, temps, source, destination, type de transfert, endpoint, octets (hex).

## 3. Lire la capture

```sh
cd re_work/captures
# tout ce qui n'est pas du flux (énumération, requêtes de contrôle) :
grep -v -E "ISOCH_TRANSFER|BULK_OR_INTERRUPT" testzireshqrk3.tsv | cut -c1-200
cd ../..
```

On y voit l'initialisation complète : `SET_CONFIGURATION 1`, `SET_INTERFACE 1 alt 1`, requêtes
vendor `0x52` (n° de série) et `0x02` (version firmware `28 01` = 1.40), puis :
- **ISO OUT 0x01** (paquets de 952 octets) : l'écran ; premier paquet = `HERCULES` + zéros, puis un
  message commençant par `SM` ;
- **Interrupt IN 0x81** (64 octets toutes les 16 ms) : boutons/molettes + un message `sm`.

## 4. Suivre l'évolution des entrées

```sh
python3 -I tools/re/in_reports.py re_work/captures/testzireshqrk3.tsv changes | head -80   # octets qui changent
python3 -I tools/re/in_reports.py re_work/captures/testzireshqrk3.tsv sm | head -20        # messages 'sm'
```

En sachant ce qu'on a fait pendant la capture (« j'ai tourné la molette 2 »), les octets qui bougent
désignent le bon champ. Le mapping exact des boutons a été confirmé plus tard en direct (fichier 05).
