# Carte de `hsm_api_core_x64.dll` (adresses Ghidra, base 0x180000000)

| Adresse | Nom proposé | Rôle |
|---------|-------------|------|
| 0x180005e60 | `dev_thread` | thread par device : CreateFile, IOCTL init, 16 lectures INT, boucle envoi |
| 0x180004ce0 | `build_content_frame` | trame contenu (tous les opcodes, cf. commands.md) |
| 0x180004540 | `send_keepalive` | SM + seq + HOST_INFO + 0x11 + fin (toutes les 500 ms) |
| 0x180015820 | `put_host_info` | opcode 0x10 |
| 0x180016570 | `sm_finalize` | longueur, padding 952, CRC16 |
| 0x180016490 | `crc_table_init` | table CRC |
| 0x180016740 | `parse_in_sm` | parse `sm` (checksum, type 1/2) |
| 0x180016ee0 | `input_dispatch` | lit l'anneau d'entrées → callback app |
| 0x180007450 | `input_ring_pop` | |
| 0x180004900 | `put_graphic_rle` | opcodes 0x35/0x36 (RLE) |
| 0x1800046a0 / 0x180004790 | `argb_to_565a` / `color_filter` | conversions couleur |
| 0x180015d10 | `put_display_settings` | 0x32 |
| 0x180015fd0 | `put_vu_style` | 0x34 (source `FUN_180003ea0`) |
| 0x180016140 | `put_bitmap_chunk` | 0x37 |
| 0x1800162e0 | `put_screen_config` | 0x38 |
| 0x180015b60 | `put_serial` | 0x21 |
| 0x180004320 | `screen_history_store` | copie des trames envoyées (pour `HSM_GetDeviceScreenContent`) |

Structures API (1er champ = taille) : DisplaySettings cb=0x10 {double brightness, u8},
AudioLevels cb=0x38, AudioChannelStyle cb=200, ScreenConfiguration cb=0x28 (9 int).
Handle device = `slot << 20 | …` (slot = `h >> 20 & 0x3f`). Shadow config copiée sous mutex
(`mutexScreenConfig`) avant chaque trame ; `HSM_LockUnlockSetChanges` = mise à jour atomique.
