"""Upper-shell parts in plan view, expressed as (cross section, length).

Cross section -1 is the left edge, +1 is the right edge. Length runs from
the nose to the heel. The curves are authored geometry for click plates,
grips and palm covers; their rasterizations are independent of reference
photographs.
"""

# Broad separate click plates with rounded rear corners. The click split
# terminates at the raised central wheel housing.
MODERN = [
    ((-.92,.047),(-.20,.010),(-.14,.024),(-.14,.117),(-.03,.225),
     (-.025,.480),(-.77,.480),(-.91,.453),(-.97,.347),(-.96,.132)),
    ((.92,.047),(.20,.010),(.14,.024),(.14,.117),(.03,.225),
     (.025,.480),(.77,.480),(.91,.453),(.97,.347),(.96,.132)),
]

VIPER = [
    ((-.91,.040),(-.16,.014),(-.12,.105),(-.15,.255),(-.18,.337),
     (-.62,.376),(-.89,.449),(-.95,.420),(-.96,.251),(-.94,.083)),
    ((.91,.040),(.16,.014),(.12,.105),(.15,.255),(.18,.337),
     (.62,.376),(.89,.449),(.95,.420),(.96,.251),(.94,.083)),
]

BASILISK = [
    ((-.86,.065),(-.15,.010),(-.10,.039),(-.09,.487),(-.21,.502),
     (-.66,.559),(-.79,.531),(-.85,.420),(-.93,.194)),
    ((.91,.050),(.22,.008),(.17,.018),(.16,.108),(.15,.409),(.30,.425),
     (.83,.344),(.90,.299),(.95,.143)),
    ((-.58,.621),(-.49,.584),(.65,.449),(.82,.450),(.84,.503),(.78,.704),
     (.82,.877),(.71,.934),(.41,.964),(-.34,.964),(-.71,.929),(-.82,.856),(-.66,.744)),
]

# This variant retains the shared tilted palm panel and adds the separate
# leading button beside the left click plate.
BASILISK_V4 = [
    ((-.86,.065),(-.15,.010),(-.10,.039),(-.09,.487),(-.21,.502),
     (-.66,.559),(-.79,.531),(-.85,.420),(-.93,.194),(-.88,.158),
     (-.54,.138),(-.47,.047)),
    *BASILISK[1:],
]

BASILISK_INSERTS = [
    ((-.98,.153),(-.86,.187),(-.78,.380),(-.75,.536),(-.67,.678),
     (-.58,.788),(-.67,.855),(-.82,.842),(-.98,.747),(-.99,.520),
     (-.95,.380)),
    ((.94,.123),(.99,.212),(.99,.408),(.975,.735),(.76,.875),(.63,.82),
     (.72,.690),(.78,.561),(.81,.437),(.91,.296)),
]

NAGA = [
    ((-.86,.064),(-.49,.032),(-.35,.124),(-.28,.233),(-.34,.359),
     (-.33,.570),(-.45,.673),(-.56,.805),(-.47,.925),(-.29,.971),
     (.30,.978),(.78,.918),(.97,.740),(.97,.433),(.92,.207),(.83,.063),
     (.26,.017),(.14,.022),(.14,.362),(.11,.459),(-.064,.460),(-.075,.052),(-.12,.018)),
]

VERTICAL_INSERTS = [
    ((-.84,.213),(-.24,.137),(-.17,.200),(-.18,.530),(-.23,.702),
     (-.34,.803),(-.74,.814),(-.90,.737),(-.94,.511)),
]

PRO_CLICK = [
    ((-.95,.109),(-.12,.020),(-.08,.039),(-.08,.193),(-.06,.364),
     (-.79,.462),(-.87,.430),(-.94,.268)),
    ((.91,.087),(.22,.021),(.11,.028),(.08,.195),(.06,.362),(.63,.433),
     (.85,.382),(.93,.210)),
]


def plates(spec):
    shape = spec["shape"]
    if spec.get("split_cap"):
        return BASILISK_V4
    if shape == "basilisk" or (shape == "basilisk-classic" and spec.get("rail")):
        return BASILISK
    if shape in ("viper-modern", "deathadder") and spec.get("top_buttons", 0) == 0:
        return MODERN
    if shape in ("viper", "cobra"):
        return VIPER
    if shape in ("naga","naga-classic") and spec.get("side_buttons", 0) > 5:
        return NAGA
    if shape == "pro-click":
        return PRO_CLICK
    return ()


def grips(spec):
    if spec["shape"] == "basilisk":
        return BASILISK_INSERTS
    if spec["shape"] == "vertical":
        return VERTICAL_INSERTS
    return ()
