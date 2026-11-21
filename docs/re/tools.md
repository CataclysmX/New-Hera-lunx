# Outils RE (`tools/re/`) – régénérer `re_work/`

Scripts Python lancés avec `python3 -I` (données extraites = non fiables, jamais exécutées).
Tout passe par des fichiers scripts (rejouables, documentés par leur en-tête `Usage:`).
Démarche complète pas à pas : [../../explanation/00-sommaire.md](../../explanation/00-sommaire.md).

```sh
python3 -I tools/re/is_unpack.py Hercules_Stream_Control_v2.5.4.exe re_work/installer
python3 -I tools/re/iscab_unpack.py re_work/installer/Disk1 re_work/app
python3 -I tools/re/is_unpack.py re_work/app/HSM.exe re_work/hsm
python3 -I tools/re/iscab_unpack.py re_work/hsm/Disk1 re_work/hsm_files
tail -c +129 re_work/hsm_files/HSM01_S32L4R7_v1_42.hsm > re_work/fw/hsm01_v1_42_body.bin
# décompilation complète -> .c (exploré avec grep / awk / sed)
/opt/ghidra/support/analyzeHeadless re_work/ghidra_proj p -import <bin> -scriptPath tools/re \
   -postScript DecompileAll.java <abs/out.c> -deleteProject
#   firmware : ajouter -processor ARM:LE:32:Cortex -loader BinaryLoader -loader-baseAddr 0x08000000
python3 -I tools/re/wsdump.py wireshark.txt > re_work/captures/wireshark.tsv
```

| Script | Usage |
|--------|-------|
| `pe_overlay.py` / `pe_imports.py [--funcs] [--exports]` / `pe_read.py file va len fmt` | inspection PE |
| `hsm01_proto.py check\|decode\|decode-v <tsv>` | CRC, décodage commandes (référence Python du protocole) |
| `in_reports.py <tsv> changes\|sm` | évolution du bloc contrôles / paquets `sm` |

Ghidra en interface graphique pour les analyses ciblées (références croisées, chaînes, renommage) ;
le gros du travail se fait sur les `.c` exportés en mode headless (`grep`, `awk`, `sed`).
