# Entrées (device → hôte, Interrupt IN 0x81)

Report 64 o = bloc contrôles + message `sm` (cf. [framing.md](framing.md)).

## Bloc contrôles (12 o) – construit par le firmware `FUN_08005ee4`, émis si changement (≥ 3 ms)
| Off | Type | Contenu |
|-----|------|---------|
| 0 | u8 | longueur du bloc = 0x0c |
| 1 | u8 | bits 0-3 = **clic molettes 1-4**, bits 4-7 = **boutons 1-4** (GPIO actifs bas, anti-rebond 20 ms, `FUN_0800e610`) |
| 2 | u8 | bit 0 = bouton **power** |
| 3-10 | 4 × i16 LE | **molettes 1-4** : accumulateurs signés (potentiomètres sans fin double curseur, `FUN_0800e128`, algorithme détaillé dans [../firmware/knobs.md](../firmware/knobs.md) : 512 pas/tour, horaire = +) → l'hôte calcule `delta = new - old` (wrap 16 bits). Rotation rapide : +5…+25 par report (16 ms) |
| 11 | u8 | **écran** : bits 0-6 = luminosité % courante, bit 7 = écran allumé (0xbc = on 60 %, 0xe4 = on 100 %) |

Mapping physique **confirmé en live** (2026-11-19, ordre molettes 1→4, boutons 1→4, clics 1→4, power) :
`u16 buttons` = clic1 0x0001, clic2 0x0002, clic3 0x0004, clic4 0x0008, bouton1 0x0010, bouton2 0x0020,
bouton3 0x0040, bouton4 0x0080, power 0x0100. Un clic molette fait souvent bouger l'encodeur de 1-4 → filtrer.

Comportements locaux du firmware (sans l'hôte) :
- power appui court (au relâchement) : luminosité +20 % (20→40→…→100→20) ✔ live ; appui long (>500 ms) :
  écran off ; appui quand off : rallume (≥ 20 %).
- power + bouton 1 : bascule un mode où les entrées ne sont plus remontées (luminosité 80/100).
- clic molette 1 + bouton 4 maintenus, double appui < 500 ms : bascule un état interne (TBD).

La DLL met ces blocs dans un anneau de 4 entrées puis appelle le **callback** passé à
`HSM_OpenDevice(index, &handle, callback, user)` avec :
`{u32 time, u16 buttons, i16 enc[4], float vuDisplayed[8] (-1 = n/a), u8 state}`.
Le callback n'est appelé que si boutons/encodeurs/état/VU changent.

TODO : mapping complet des bits boutons (décompilation `Stream-control.exe` en cours).

## Message `sm` type 2 = statut (≈ toutes les 16 ms)
Exemple : `sm 21 00 e8 d1 02 | 00 00 3c 01 09 00 5a 03 02 | 6e c2 a7 07 07 9b 00 00 03 | 00 00 00 00 | 03 0e 0c ff 6e 00 10 00 04 09 04 ...`
Champs DLL (`DeviceStatus` log) : PkId=@4, Error u16 @7 (bits fw : 0x08 CRC, 0x10 signature, 0x20 longueur),
ErrorCount, AvgSpeed, s/n 9 o, usFactoryWordCode, usLedStates, ucLastInvCmd, Render ms,
SplashScreen max ms, FlashProgrammingStatus. Offsets exacts : voir `FUN_180016740`.

## Message `sm` type 1 = infos device (envoyé au démarrage ; l'hôte répond `0x14`)
Capturé : `01 28 01 | 02 6e c2 a7 07 07 9b 00 00 | 03 00 | e0 01 | 10 01 | 04 00 04 02 | 20 00 20 00 | 6e 00 10 00 | 04 09 04 | 00…`
| Off sm | Valeur | Sens (champ slot DLL) |
|--------|--------|------------------------|
| 7 | 01 | type device (1 = "Hercules Stream 100") |
| 8-9 | 0x0128 | version firmware 1.40 (+0x844) |
| 10-18 | 9 o | n° de série (rev `02` + 8 o) |
| 19-20 | 3 | ? (+0x846) |
| 21-22 | **480** | largeur écran (+0x84c) |
| 23-24 | **272** | hauteur écran (+0x84e) |
| 25 / 26 / 27 / 28 | 4 / 0 / 4 / 2 | audioSourceCount / ? / encoderCount / vuMeterStyleCount (probables) |
| 29-30 / 31-32 | 32 / 32 | audioSrcIconSize 32×32 (= icônes `0x35`) |
| 33-34 / 35-36 | 110 / 16 | actionLabelSize 110×16 (= labels `0x36`) |
| 37 / 38 / 39 | 4 / **9** / **4** | ? / buttonCount (8 + spécial) / ledCount (borne de la boucle 0x30) |
Noms issus du dialogue debug de l'app (`m_ledCount`, `m_encoderCount`, `m_buttonCount`,
`m_audioSourceCount`, `m_viewMeterStyleCount`, `m_audioSrcIconSize`, `m_actionLabelSize`, `m_screen`,
`m_firmware`, `m_vid/m_pid`, `m_symLink`) ; association champ↔octet = probable, cohérente avec le firmware.
Struct `HSM_GetDeviceInfo` : cb = 0x640 (nom, VID/PID, chemin, instance, puis ces u16).

## Requêtes vendor (control IN, cf. [usb.md](usb.md))
- `0x52 wValue=0` → 2 o (`01 02`) ; `0x52 wValue=1` → `[8][s/n 8 o]`
- `0x02` → 2 o (`28 01`), relu périodiquement.
