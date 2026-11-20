import sys
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
src, out, opsz = sys.argv[1], sys.argv[2], int(sys.argv[3])
f = TTFont(src)
inst = instantiateVariableFont(f, {"opsz": opsz}, updateFontNames=False)
family = f"Inter Opsz{opsz}"
name = inst["name"]
for rec in list(name.names):
    if rec.nameID in (1, 16):
        name.setName(family, rec.nameID, rec.platformID, rec.platEncID, rec.langID)
    elif rec.nameID in (4,):
        name.setName(f"{family} Regular", rec.nameID, rec.platformID, rec.platEncID, rec.langID)
    elif rec.nameID == 6:
        name.setName(f"InterOpsz{opsz}-Regular", rec.nameID, rec.platformID, rec.platEncID, rec.langID)
inst.save(out)
