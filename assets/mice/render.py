"""Build Pointrill's original mouse illustrations from parametric meshes.

Run from the repository root: blender -b -t 8 -P assets/mice/render.py
Pass model IDs after '--' to render a subset. Blender and ImageMagick are
needed to regenerate illustrations. The app embeds the resulting WebP files.
"""
import json
import math
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

import bpy
from mathutils import Matrix, Vector


ROOT = Path(__file__).resolve().parent
STATIONS = (0, .025, .10, .22, .38, .55, .70, .84, .95, .99, 1)
# Cross-sections run from the buttons to the heel. Widths are half widths,
# normalized against the shell width, and heights describe the centerline.
SHAPES = {
    "abyssus": ((.08,.49,.89,.93,.88,1,.99,.90,.66,.32,.06), (.18,.39,.48,.55,.68,.9,1,.84,.50,.24,.08)),
    "atheris": ((.07,.42,.78,.91,.96,.96,1,.92,.64,.28,.06), (.19,.40,.52,.61,.81,.98,1,.84,.50,.24,.08)),
    "basilisk": ((.07,.49,.91,.95,.88,.85,.99,1,.71,.32,.06), (.17,.39,.49,.57,.77,.96,1,.84,.47,.24,.08)),
    "basilisk-classic": ((.07,.46,.83,.87,.79,.86,1,.97,.73,.36,.06), (.17,.37,.49,.58,.77,.95,1,.83,.50,.24,.08)),
    "cobra": ((.07,.50,.94,1,.85,.85,.97,1,.75,.32,.06), (.17,.32,.42,.51,.66,.89,1,.87,.52,.24,.08)),
    "deathadder": ((.07,.49,.94,1,.86,.84,.94,.99,.72,.33,.06), (.17,.37,.46,.50,.62,.90,1,.91,.54,.24,.08)),
    "deathadder-classic": ((.07,.55,.98,1,.80,.76,.96,1,.75,.32,.06), (.17,.38,.52,.55,.68,.92,1,.86,.51,.24,.08)),
    "diamondback": ((.08,.50,.95,.96,.79,.78,.91,1,.73,.31,.06), (.16,.33,.46,.56,.69,.93,1,.83,.49,.24,.08)),
    "imperator": ((.07,.47,.85,.93,.89,.87,.97,1,.75,.34,.06), (.18,.43,.51,.62,.79,.97,1,.85,.48,.24,.08)),
    "lancehead": ((.08,.49,.87,.89,.80,.87,1,1,.78,.36,.06), (.18,.38,.54,.59,.71,.89,1,.87,.54,.24,.08)),
    "mamba": ((.06,.49,.91,.95,.85,.85,.94,1,.76,.35,.06), (.17,.40,.51,.58,.71,.93,1,.87,.54,.24,.08)),
    "naga": ((.07,.48,.83,.90,.99,1,1,.91,.64,.33,.06), (.18,.36,.50,.58,.79,.96,1,.84,.53,.25,.08)),
    "naga-classic": ((.07,.44,.78,.90,.95,.98,1,.97,.70,.35,.06), (.18,.38,.53,.63,.82,.99,1,.86,.52,.24,.08)),
    "orochi": ((.06,.41,.75,.92,.99,1,.98,.92,.69,.31,.06), (.17,.35,.51,.62,.80,1,.97,.80,.47,.24,.08)),
    "orochi-classic": ((.08,.55,.88,.94,.97,.98,1,.93,.68,.31,.06), (.17,.38,.54,.61,.80,.98,1,.82,.52,.24,.08)),
    "ouroboros": ((.08,.62,.96,1,.97,.78,.83,.95,.79,.46,.13), (.17,.36,.53,.64,.76,.94,1,.88,.60,.31,.13)),
    "pro-click": ((.07,.46,.80,.90,.94,.96,1,.97,.76,.36,.06), (.16,.36,.50,.58,.77,.98,1,.83,.47,.24,.08)),
    "taipan": ((.06,.49,.86,.98,.87,.85,.96,1,.66,.31,.06), (.17,.38,.51,.62,.73,.91,1,.87,.48,.24,.08)),
    "vertical": ((.07,.40,.75,.94,.96,1,1,.94,.72,.34,.06), (.18,.37,.57,.75,.92,1,.96,.77,.46,.24,.08)),
    "viper": ((.07,.49,.93,1,.87,.79,.91,1,.70,.32,.06), (.15,.36,.48,.54,.62,.81,1,.90,.50,.24,.08)),
    "viper-modern": ((.07,.51,.95,.98,.90,.86,.92,1,.73,.32,.06), (.16,.34,.45,.52,.60,.80,1,.90,.51,.24,.08)),
}
FINISHES = {
    "graphite": (.026, .034, .041),
    "night": (.018, .023, .029),
    "black-gloss": (.021, .024, .027),
    "silver": (.14, .16, .19),
    "white": (.69, .74, .79),
    "green": (.034, .15, .083),
    "red": (.34, .023, .032),
    "blue": (.048, .105, .24),
}
LIGHTS = {
    "green": (.11, 1., .033),
    "cyan": (.035, .68, 1.),
    "red": (1., .07, .026),
    "violet": (.44, .13, 1.),
    "white": (.62, .76, 1.),
}


def sample(values, t):
    i = next(i for i in range(len(STATIONS)-1) if t <= STATIONS[i+1])
    x0, x1 = STATIONS[i:i+2]
    a, b = values[i:i+2]
    before, after = max(0, i-1), min(i+2, len(values)-1)
    m = (b-values[before]) / (x1-STATIONS[before]) * (x1-x0)
    n = (values[after]-a) / (STATIONS[after]-x0) * (x1-x0)
    u = (t-x0) / (x1-x0)
    return a + u * (m + u * (3*(b-a)-2*m-n + u*(2*(a-b)+m+n)))


def material(name, color, rough=.36, metal=.16, emission=0, texture=False):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    shader = mat.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (*color, 1)
    shader.inputs["Roughness"].default_value = rough
    shader.inputs["Metallic"].default_value = metal
    if emission:
        shader.inputs["Emission Color"].default_value = (*color, 1)
        shader.inputs["Emission Strength"].default_value = emission
    if texture:
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        noise = nodes.new("ShaderNodeTexNoise")
        noise.inputs["Scale"].default_value = 175
        bump = nodes.new("ShaderNodeBump")
        bump.inputs["Strength"].default_value = .20
        bump.inputs["Distance"].default_value = .003
        links.new(noise.outputs["Fac"], bump.inputs["Height"])
        links.new(bump.outputs["Normal"], shader.inputs["Normal"])
    return mat


def mesh(name, vertices, faces, mat, smooth=True):
    geo = bpy.data.meshes.new(name)
    geo.from_pydata(vertices, [], faces)
    geo.update()
    obj = bpy.data.objects.new(name, geo)
    bpy.context.collection.objects.link(obj)
    geo.materials.append(mat)
    for face in geo.polygons:
        face.use_smooth = smooth
    return obj


def stroke(name, points, thickness, mat):
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    curve.bevel_depth = thickness
    curve.bevel_resolution = 3
    spline = curve.splines.new("POLY")
    spline.points.add(len(points)-1)
    for p, point in zip(spline.points, points):
        p.co = (*point, 1)
    obj = bpy.data.objects.new(name, curve)
    bpy.context.collection.objects.link(obj)
    curve.materials.append(mat)
    return obj


def block(name, position, dims, radius, mat, rotation=(0, 0, 0)):
    bpy.ops.mesh.primitive_cube_add(size=1, location=position)
    obj = bpy.context.object
    obj.name = name
    obj.dimensions = dims
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.rotation_euler = rotation
    obj.data.materials.append(mat)
    bevel = obj.modifiers.new("Edge radius", "BEVEL")
    bevel.width, bevel.segments = radius, 4
    obj.modifiers.new("Face normals", "WEIGHTED_NORMAL")
    return obj


def cylinder(name, center, radius, width, mat, rotation=(0, math.pi/2, 0)):
    bpy.ops.mesh.primitive_cylinder_add(vertices=44, radius=radius, depth=width,
                                      location=center, rotation=rotation)
    obj = bpy.context.object
    obj.name = name
    obj.data.materials.append(mat)
    for face in obj.data.polygons:
        face.use_smooth = len(face.vertices) == 4
    bevel = obj.modifiers.new("Rim", "BEVEL")
    bevel.width, bevel.segments = .005, 3
    obj.modifiers.new("Face normals", "WEIGHTED_NORMAL")


class Mouse:
    def __init__(self, spec):
        self.spec = spec
        self.width, self.height = spec["width"]/2, spec["height"]
        self.widths, self.heights = SHAPES[spec["shape"]]
        self.lean = spec.get("lean", 0)
        self.shelf = spec.get("shelf", 0)
        self.handed = 1 if spec.get("left_handed") else -1
        self.wheel_y = spec.get("wheel_y", .29)
        self.wheel_width = spec.get("wheel_width", .078)
        self.finish = spec.get("finish", "graphite")
        color = FINISHES[self.finish]
        self.shell = material("Molded shell", color, .245 if self.finish.endswith("gloss") else .38, texture=True)
        self.base = material("Lower shell", tuple(.36*c for c in color), .44)
        self.seam = material("Recess", (.010, .013, .015))
        self.rubber = material("Rubber", (.022, .026, .029), .65, .04, texture=True)
        self.metal = material("Wheel alloy", (.22, .25, .29), .26, .68)
        self.keys = material("Keys", tuple(.70*c for c in color), .34, texture=True)
        self.led = material("Light pipe", LIGHTS[spec.get("light", "green")], .26, .02, .94)

    def span(self, t):
        return self.width * max(.02, sample(self.widths, t))

    def height_at(self, t):
        gain = self.spec.get("hump", 0)
        return self.height * (sample(self.heights, t) + gain * math.sin(math.pi*t)**3)

    def side(self, t, u):
        shelf = self.shelf * math.exp(-((t-.50)/.19)**2) * max(-u, 0)**3
        midline = .23*self.lean*math.sin(math.pi*t) - .022*self.lean
        return (midline + self.span(t)*u - shelf) * self.handed

    def point(self, t, u, lift=0):
        span = self.height_at(t)
        crown = (1-min(1, abs(u))**2.25)**.58
        z = .032 + span*(.23 + .77*crown - self.lean*u*.72)
        return (self.side(t, u), t-.5, z+lift)

    def outline(self, z, scale, first=0, last=1):
        return [(self.side(first+(last-first)*i/64, side)*scale,
                 first+(last-first)*i/64-.5, z)
                for side, steps in ((1, range(65)), (-1, range(64, -1, -1)))
                for i in steps]

    def body(self, cutout):
        steps, across = 132, 64
        points = []
        faces = []
        for i in range(steps+1):
            t = i/steps
            # Top grid, then a rounded return around the underside.
            points.extend(self.point(t, 2*j/across-1) for j in range(across+1))
            for u, scale, z in ((1,.99,.045), (1,.92,.018), (-1,.92,.018), (-1,.99,.045)):
                x, y, _ = self.point(t, u)
                points.append((x*scale, y, z))
        stride = across+5
        for i in range(steps):
            for j in range(stride):
                t, u = (i+.5)/steps, 2*(j+.5)/across-1
                if j < across and t > .48 and any(triangle_contains((u, t), triangle) for triangle in cutout):
                    continue
                a, b = i*stride+j, i*stride+(j+1)%stride
                faces.append((a,b,b+stride,a+stride))
        faces.extend((tuple(reversed(range(stride))), tuple(range(steps*stride,len(points)))))
        if self.handed < 0:
            faces = [tuple(reversed(face)) for face in faces]
        shell = mesh("Shell", points, faces, self.shell)
        shell.data.materials.append(self.base)
        for face in shell.data.polygons:
            if all(v % stride >= across for v in face.vertices):
                face.material_index = 1

    def controls(self):
        t = self.wheel_y
        x, y, z = self.point(t, 0)
        angle = self.spec.get("wheel_tilt", 0) * self.handed
        axis = Vector((math.cos(angle), 0, math.sin(angle)))
        rotation = (0, math.pi/2-angle, 0)
        self.wheel_slot(t)
        radius = self.spec.get("wheel_radius", .064)
        center = Vector((x, y, z-radius*.46))
        alloy = self.metal if self.spec.get("alloy") else self.keys
        rim = self.led if self.spec.get("wheel_light") else alloy
        cylinder("Wheel edge", center, radius, self.wheel_width, rim, rotation)
        cylinder("Wheel tread", center, radius+.004, self.wheel_width*.74, self.rubber, rotation)
        for i in range(40):
            a = i*math.tau/40
            if math.cos(a) > .29:
                lift = radius + .004
                centerline = center + Vector((-math.sin(angle)*math.cos(a)*lift,
                                             math.sin(a)*lift, math.cos(angle)*math.cos(a)*lift))
                stroke("Knurl", [centerline-axis*self.wheel_width*.33,
                                  centerline+axis*self.wheel_width*.33], .0019, self.metal)

        for i in range(self.spec.get("top_buttons", 0)):
            t = self.wheel_y + .125 + .060*i
            x,y,z = self.point(t, 0)
            block("Mode button", (x,y,z+.004), (.036,.037,.015), .007, self.keys,
                  rotation=(.40, -self.lean, 0))
        if self.spec.get("status"):
            x,y,z = self.point(self.wheel_y+.08, .35)
            stroke("Status LED", [(x-.009,y,z+.004),(x+.009,y,z+.004)], .003, self.led)

    def wheel_slot(self, t):
        points = [self.point(t+.079*math.cos(i*math.tau/64),
                              self.wheel_width/self.width*.66*math.sin(i*math.tau/64), .002)
                  for i in range(64)]
        mesh("Wheel recess", points, [tuple(range(64))], self.seam, False)
        stroke("Wheel bezel", points+[points[0]], .003, self.keys)

    def seams(self):
        depth = self.spec.get("split", .43)
        stroke("Primary split", [self.point(.018+i*(self.wheel_y-.090)/32,0,.002)
                                 for i in range(33)], .0033, self.seam)
        if self.spec.get("continuous_shell"):
            stroke("Center groove", [self.point(self.wheel_y+.07+i*.004,0,.002)
                                     for i in range(41)], .0025, self.seam)
        else:
            for side in (-1,1):
                stroke("Primary key seam", [
                    self.point(depth + .052*side*self.lean - .044*abs(u)
                               + self.spec.get("cut", 0)*u*u, u, .002)
                    for u in (side*i/39 for i in range(2,40))
                ], .004, self.seam)
        if self.spec.get("rail"):
            for side in (-1, 1):
                stroke("Side shell seam", [self.point(.19+i*.60/46, side*.94, .001)
                                           for i in range(47)], .0028, self.seam)
        mode = self.spec.get("edge", "none")
        if mode != "none":
            edge = self.outline(.035, .968, .07 if mode == "sides" else .50)
            if mode == "sides":
                for part in (edge[13:61], edge[69:117]):
                    stroke("Side light", part, .005, self.led)
            else:
                stroke("Heel light", edge, .006, self.led)

    def thumb_controls(self):
        count = self.spec.get("side_buttons", 2)
        z = self.height_at(.43)*.41+.04
        if count in (6, 7, 12):
            x = self.side(.46, -1)
            block("MMO button panel", (x*.96,-.035,z-.008), (.065,.34,.11),
                  .024, self.rubber, rotation=(0, .57*self.handed, 0))
            if count == 12:
                positions = [(row*.073-.145, tier*.031-.032)
                             for row in range(4) for tier in range(3)]
            else:
                positions = [(math.sin(i*math.tau/count)*.10,
                              math.cos(i*math.tau/count)*.047) for i in range(count)]
            for y, dz in positions:
                button_x = x-self.handed*(.025-.62*dz)
                block("Thumb key", (button_x, y-.028,z+dz), (.035,.044,.024),
                      .007, self.keys, rotation=(0, .57*self.handed, 0))
        elif count:
            for side in (-1, 1) if self.spec.get("both_sides") else (-1,):
                for i in range(count):
                    t = .35+i*.10
                    x, y, z = self.point(t, side*.95, -.003)
                    block("Thumb key", (x+side*self.handed*.004,y,z), (.035,.079,.027),
                          .009, self.keys, rotation=(.23, -.25*side*self.handed, 0))
        if self.spec.get("clutch"):
            x,y,z = self.point(.26,-.96)
            block("Clutch paddle", (x-self.handed*.024,y,z-.012), (.065,.083,.025),
                  .010, self.keys, rotation=(.10, .16*self.handed, .23*self.handed))
        if self.spec.get("grip"):
            for i in range(12):
                t = .43+i*.025
                x,y,z = self.point(t,-.99)
                stroke("Grip rib", [(x,y,z), (x*.987,y+.022,max(.055,z-.021))], .0015, self.rubber)

    def draw(self):
        cutouts = []
        if self.spec.get("lattice"):
            for row, t in enumerate((.54,.665,.79,.885)):
                height = .05 if row < 2 else .038
                for column, u in enumerate((-.57,-.19,.19,.57)):
                    sign = (-1)**(row+column)
                    width = .157 if row < 3 else .138
                    cutouts.append(((u-width,t-height*sign),(u+width,t-height*sign),(u,t+height*sign)))
        self.body(cutouts)
        for triangle in cutouts:
            edge = [self.point(t+(ty-t)*i/12, u+(ux-u)*i/12, .001)
                    for (u,t),(ux,ty) in zip(triangle, triangle[1:]+triangle[:1])
                    for i in range(12)]
            stroke("Lattice bevel", edge+[edge[0]], .004, self.keys)
        self.seams()
        self.controls()
        self.thumb_controls()


def triangle_contains(point, triangle):
    signs = [(point[0]-x)*(ny-y) - (point[1]-y)*(nx-x)
             for (x,y),(nx,ny) in zip(triangle, triangle[1:]+triangle[:1])]
    return min(signs) >= 0 or max(signs) <= 0


def dongle():
    rubber = material("Rubber feet", (.010,.015,.019), .55)
    cover = material("Receiver shell", (.065,.075,.080), .38, texture=True)
    port = material("USB recess", (.007,.009,.012), .46)
    led = material("Link indicator", LIGHTS["green"], .28, emission=1.2)
    block("Receiver base", (0,0,.04), (.47,.75,.075), .012, rubber)
    block("Receiver cover", (0,0,.165), (.47,.75,.21), .020, cover)
    block("USB port", (0,-.375,.143), (.155,.009,.069), .003, port)
    block("Status LED", (0,.240,.272), (.066,.012,.004), .002, led)


def dock(pro):
    casing = material("Charging dock", (.027,.034,.043), .36, texture=True)
    light = material("Dock light", LIGHTS["green"], .26, emission=.98)
    contact = material("Contact", (.41,.26,.09), .25, .75)
    if pro:
        # Two half circles and straight sides form the charging platform.
        outline = [(r*math.cos(i*math.pi/40), offset+r*math.sin(i*math.pi/40))
                   for r,offset,steps in ((.285,.14,range(41)),(.285,-.14,range(40,81)))
                   for i in steps]
        rings = [[(x*scale,y*scale,z+slope*y) for x,y in outline]
                 for scale,z,slope in ((.96,.028,0),(1,.052,0),(1,.092,0),
                                       (.99,.24,.34),(.94,.252,.34))]
        count = len(outline)
        vertices = [point for ring in rings for point in ring]
        faces = [(row*count+i,row*count+(i+1)%count,(row+1)*count+(i+1)%count,(row+1)*count+i)
                 for row in range(len(rings)-1) for i in range(count)]
        faces.append(tuple(range((len(rings)-1)*count,len(vertices))))
        body = mesh("Charging platform", vertices, faces, casing, False)
        body.modifiers.new("Face normals", "WEIGHTED_NORMAL")
        stroke("Dock light ring", rings[1]+[rings[1][0]], .010, light)
        block("USB-C socket", (0,.419,.187), (.112,.010,.038), .008,
              material("Port", (.006,.008,.012)))
    else:
        light_base = block("Dock light ring", (0,0,.045), (.54,.67,.053), .035, light)
        light_base.scale.x = .96
        polygon = [(-.245,-.29),(.245,-.29),(.245,.17),(.14,.34),(-.14,.34),(-.245,.17)]
        lower = [(x,y,.062) for x,y in polygon]
        upper = [(x*.72,y*.86,.205+.20*y) for x,y in polygon]
        sides = [(i,(i+1)%6,(i+1)%6+6,i+6) for i in range(6)]
        body = mesh("Dock pedestal", lower+upper, sides+[tuple(range(6,12))], casing, False)
        bevel = body.modifiers.new("Pedestal bevel", "BEVEL")
        bevel.width, bevel.segments = .018, 4
        body.modifiers.new("Face normals", "WEIGHTED_NORMAL")
        block("USB-A receiver port", (0,.17,.241), (.135,.075,.015), .006,
              material("Port", (.006,.008,.012)), rotation=(.18,0,0))
        for x in (-.06,.06):
            cylinder("Charging pin", (x,-.043,.218), .019, .042, contact, rotation=(.15,0,0))

def aim(obj, point):
    obj.rotation_euler = (Vector(point)-obj.location).to_track_quat("-Z","Y").to_euler()


def studio():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 12
    scene.cycles.use_denoising = True
    scene.cycles.max_bounces = 4
    scene.render.film_transparent = True
    scene.render.resolution_x = 412
    scene.render.resolution_y = 512
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "AgX"
    world = scene.world
    world.use_nodes = True
    world.node_tree.nodes.get("Background").inputs[0].default_value = (.32,.38,.43,1)
    world.node_tree.nodes.get("Background").inputs[1].default_value = .36
    bpy.ops.object.camera_add()
    scene.camera = bpy.context.object
    scene.camera.data.type = "ORTHO"
    scene.camera.data.ortho_scale = 1.21
    for name, position, size, energy, color in (
        ("White softbox", (-1,-1.0,2.25), 1.35, 120, (.84,.91,1)),
        ("Tall bounce", (1.25,-.28,1.4), 1.25, 68, (.91,.96,1)),
        ("Mint rim", (.63,1.0,.91), .93, 54, (.63,1,.78)),
        ("Rim fill", (-.91,.78,.77), .77, 23, (.70,.82,1)),
    ):
        bpy.ops.object.light_add(type="AREA", location=position)
        light = bpy.context.object
        light.name = name
        light.data.size, light.data.energy, light.data.color = size, energy, color
        aim(light, (0,0,.1))
    return scene


def render(models):
    scene = studio()
    studio_names = {obj.name for obj in scene.objects}
    with TemporaryDirectory(prefix="pointrill-render-") as temp:
        png = Path(temp)/"render.png"
        for slug, spec in models.items():
            for obj in list(scene.objects):
                if obj.name not in studio_names:
                    bpy.data.objects.remove(obj, do_unlink=True)
            bpy.data.orphans_purge()
            camera = scene.camera
            handed = -1 if spec.get("left_handed") else 1
            mouse_side = 1.84 if spec.get("side_buttons", 0) > 5 else .96
            camera.location = (mouse_side*handed, -.59, 2.30)
            back = (camera.location-Vector((0,0,.14))).normalized()
            right = Vector((-.25*handed,-1,0)).cross(back).normalized()
            camera.rotation_euler = Matrix((right, back.cross(right), back)).transposed().to_euler()
            if spec["shape"] == "receiver":
                dongle()
            elif spec["shape"] in ("dock", "dock-pro"):
                dock(spec["shape"] == "dock-pro")
            else:
                Mouse(spec).draw()
            scene.render.filepath = str(png)
            bpy.ops.render.render(write_still=True)
            subprocess.run(["magick", str(png), "-trim", "+repage",
                            "-quality", "88", "-define", "webp:alpha-quality=100",
                            str(ROOT/"renders"/f"{slug}.webp")], check=True)
            print(f"Rendered {slug}", flush=True)


if __name__ == "__main__":
    entries = json.loads((ROOT/"catalog.json").read_text())
    requested = sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else entries
    (ROOT/"renders").mkdir(exist_ok=True)
    render({slug: entries[slug] for slug in requested})
