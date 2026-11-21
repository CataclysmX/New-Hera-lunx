# Framing HSM01 (validé sur captures : 3305/3305 CRC OK, 6483/6483 checksum OK)

Code de référence : `tools/re/hsm01_proto.py` (`build_frame`, `sm_crc`, `in_checksum_ok`).

## Hôte → device (ISO OUT 0x01)
Buffer = N × 952 octets (N ≤ 10), envoyé en un transfert ISO de N paquets.
```
paquet 0 : "HERCULES" + 944 × 0x00
paquet 1…: message SM, puis 0x00 jusqu'au multiple de 952
```
Message SM (DLL `FUN_180004540` + `FUN_180016570`) :
| Off | Taille | Contenu |
|-----|--------|---------|
| 0 | 2 | `'S' 'M'` |
| 2 | 2 | longueur u16 LE **de tout le message** (en-tête inclus) |
| 4 | 2 | CRC16 LE sur `msg[0:4] + msg[6:len]` |
| 6 | 2 | n° de séquence u16 LE (+1 par trame) |
| 8 | … | suite de commandes → [commands.md](commands.md), terminée par `0x00` (ou par la longueur) |

CRC16 : algo "reflected" table-driven, init 0, `crc = (crc>>8) ^ T[(crc^b)&0xff]`.
Table générée par `FUN_180016490` (variante poly 0x8005 non standard) → voir `_make_table()`.

Keep-alive (toutes les 500 ms si rien d'autre) = `SM` + seq + `HOST_INFO(0x10)` + `11 00` (len 0x22).

Côté firmware (`FUN_08008f10`) : anneau de 34 paquets ISO ; contrôle longueur 9..0x2C9F, CRC, signature.
Bits d'erreur remontés dans le statut : 0x08 CRC, 0x10 signature, 0x20 longueur, 0x40 opcode inconnu.

## Device → hôte (Interrupt IN 0x81, 64 o)
```
[0]      n = longueur bloc contrôles (octet n inclus)   (ex. 0x0c)
[1..n-1] état des contrôles → [inputs.md](inputs.md)
[n..]    message 'sm' (minuscules)
```
Message `sm` (DLL `FUN_180016740`) :
| Off | Contenu |
|-----|---------|
| 0-1 | `'s' 'm'` |
| 2 | longueur (u8, en-tête inclus) |
| 4 | PkId (compteur) |
| 6 | type : 1 = infos device, 2 = statut |
| … | checksum : **somme de tous les octets = 0 mod 256** |

Type 2 (statut) : `Error` u16 @7, `ErrorCount` @11-12, … s/n 9 o, `usFactoryWordCode`, `usLedStates`,
`ucLastInvCmd`, temps de rendu, `FlashProgrammingStatus` (détails : [inputs.md](inputs.md)).
