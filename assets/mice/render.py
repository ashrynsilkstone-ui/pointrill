"""Build Pointrill's original mouse illustrations from parametric meshes.

Run from the repository root: blender -b -t 8 -P assets/mice/render.py
Pass model IDs after '--' to render a subset. Blender and ImageMagick are
needed to regenerate illustrations. The app embeds the resulting WebP files.

Each measured plan has paired left/right edge samples from nose to heel,
normalized by shell width. The height profiles and component meshes build
the actual surfaces from those measurements.
"""
import json
import math
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

import bpy
from mathutils import Matrix, Vector
from mathutils.geometry import tessellate_polygon


ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
sys.dont_write_bytecode = True
import deck

OUTLINES = json.loads((ROOT/"outlines.json").read_text())
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
    "graphite": (.019, .022, .026),
    "night": (.013, .016, .020),
    "black-gloss": (.021, .024, .027),
    "silver": (.14, .16, .19),
    "white": (.69, .74, .79),
    "green": (.026, .086, .050),
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
    shader.inputs["Specular IOR Level"].default_value = .22
    if emission:
        shader.inputs["Emission Color"].default_value = (*color, 1)
        shader.inputs["Emission Strength"].default_value = emission
    if texture:
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        noise = nodes.new("ShaderNodeTexNoise")
        noise.inputs["Scale"].default_value = 175
        bump = nodes.new("ShaderNodeBump")
        bump.inputs["Strength"].default_value = .26
        bump.inputs["Distance"].default_value = .0016
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
    def __init__(self, slug, spec):
        self.spec = spec
        self.plan = OUTLINES.get(slug)
        self.width, self.height = spec["width"]/2, spec["height"]
        self.plans = deck.plates(spec)
        self.grips = deck.grips(spec)
        self.widths, self.heights = SHAPES[spec["shape"]]
        self.lean = spec.get("lean", 0)
        self.shelf = spec.get("shelf", 0)
        self.handed = 1 if spec.get("left_handed") else -1
        self.wheel_y = spec.get("wheel_y", .29)
        self.wheel_width = spec.get("wheel_width", .078)
        self.wheel_x = spec.get("wheel_x", 0)
        self.finish = spec.get("finish", "graphite")
        color = FINISHES[self.finish]
        self.shell = material("Molded shell", color, .31 if self.finish.endswith("gloss") else .46, texture=True)
        self.deck = material("Click plates and palm cover", tuple(c*1.15 for c in color), .52, .06, texture=True)
        self.base = material("Lower shell", tuple(.41*c for c in color), .47)
        self.seam = material("Recess", (.004, .005, .007), .62)
        self.rubber = material("Rubber", (.012, .014, .016), .71, .04, texture=True)
        self.metal = material("Wheel alloy", (.102, .115, .127), .32, .55)
        self.keys = material("Keys", tuple(.70*c for c in color), .34, texture=True)
        self.led = material("Light pipe", LIGHTS[spec.get("light", "green")], .26, .02, .94)
        self.wheel_housing = material("Wheel well", (.004,.005,.007), .52, .04)

    def span(self, t):
        return self.width * max(.02, sample(self.widths, t))

    def height_at(self, t):
        gain = self.spec.get("hump", 0)
        return self.height * (sample(self.heights, t) + gain * math.sin(math.pi*t)**3)

    def side(self, t, u):
        if self.plan:
            left, right = self.plan
            i = min(int(t*(len(left)-1)), len(left)-2)
            f = t*(len(left)-1)-i
            lo = left[i]*(1-f)+left[i+1]*f
            hi = right[i]*(1-f)+right[i+1]*f
            return -self.width*(lo+hi+(hi-lo)*u)
        shelf = self.shelf * math.exp(-((t-.50)/.19)**2) * max(-u, 0)**3
        midline = .23*self.lean*math.sin(math.pi*t) - .022*self.lean
        return (midline + self.span(t)*u - shelf) * self.handed

    def point(self, t, u, lift=0):
        span = self.height_at(t)
        crown = (1-min(1, abs(u))**2.85)**.53
        flatness = .62 - .22*min(1, max(0,(t-.33)*3.5))
        lean = self.lean if self.handed < 0 else -self.lean
        z = .027 + span*(flatness + (1-flatness)*crown + lean*u*.56)
        if self.spec["shape"] == "vertical":
            ridge = math.exp(-((u+.13)/.42)**2)
            z = .027+span*(.26+.74*ridge)*(.56+.44*crown)
        return (self.side(t, u), t-.5, z+lift)

    def body(self, cutout):
        steps, across = 132, 64
        points = []
        faces = []
        light = self.spec.get("edge", "none")
        heel_start = self.spec.get("light_start", .56)
        for i in range(steps+1):
            t = i/steps
            lit = light=="heel" and heel_start < t < .989 or light=="sides" and .16 < t < .90
            # Top grid, then a rounded return around the underside.
            points.extend(self.point(t, 2*j/across-1) for j in range(across+1))
            for u, scale, z in ((1,.994,.046),(1,.971,.037),(1,.950,.029),(1,.920,.018),
                                (-1,.920,.018),(-1,.950,.029),(-1,.971,.037),(-1,.994,.046)):
                x, y, roof = self.point(t, u)
                if lit and z in (.037,.029):
                    scale = 1.015 if z==.037 else 1.006
                points.append((x*scale, y, min(z, roof*.92)))
        stride = across+9
        for i in range(steps):
            for j in range(stride):
                t, u = (i+.5)/steps, 2*(j+.5)/across-1
                if j < across and t > .48 and any(triangle_contains((u, t), triangle) for triangle in cutout):
                    continue
                a, b = i*stride+j, i*stride+(j+1)%stride
                faces.append((a,b,b+stride,a+stride))
        faces.extend((tuple(reversed(range(stride))), tuple(range(steps*stride,len(points)))))
        if self.plan or self.handed < 0:
            faces = [tuple(reversed(face)) for face in faces]
        shell = mesh("Shell", points, faces, self.shell)
        shell.data.materials.append(self.base)
        shell.data.materials.append(self.led)
        for face in shell.data.polygons:
            if all(v % stride >= across for v in face.vertices):
                face.material_index = 1
            # Light guide molded into the lower shell, following the casing.
            stations = {v % stride for v in face.vertices}
            if stations not in ({across+2,across+3},{across+6,across+7}):
                continue
            t = sum(points[v][1]+.5 for v in face.vertices)/len(face.vertices)
            if (light=="heel" and heel_start < t < .989
                    or light=="sides" and .16 < t < .90):
                face.material_index = 2

    def controls(self):
        t = self.wheel_y
        x, y, z = self.point(t, self.wheel_x)
        angle = self.spec.get("wheel_tilt", 0) * self.handed
        axis = Vector((math.cos(angle), 0, math.sin(angle)))
        rotation = (0, math.pi/2-angle, 0)
        self.wheel_slot(t)
        radius = self.spec.get("wheel_radius", .064)
        burial = .22 if self.spec["shape"] == "vertical" else .52
        center = Vector((x, y, z-radius*burial))
        alloy = self.metal if self.spec.get("alloy") and self.spec["shape"] != "vertical" else self.keys
        rim = self.led if self.spec.get("wheel_light") else alloy
        cylinder("Wheel edge", center, radius, self.wheel_width, rim, rotation)
        tread_width = self.wheel_width*(.84 if self.spec.get("wheel_light") else .91)
        cylinder("Wheel tread", center, radius+.004, tread_width, self.rubber, rotation)
        grip = material("Wheel tread ridge", (.022,.026,.030), .64)
        for i in range(40):
            a = i*math.tau/40
            if math.cos(a) > .29:
                lift = radius + .004
                centerline = center + Vector((-math.sin(angle)*math.cos(a)*lift,
                                             math.sin(a)*lift, math.cos(angle)*math.cos(a)*lift))
                stroke("Knurl", [centerline-axis*self.wheel_width*.33,
                                  centerline+axis*self.wheel_width*.33], .00165, grip)

        keys = self.spec.get("mode_keys",
                             [(self.wheel_x,self.wheel_y+.111+.048*i,.044,.034,0)
                              for i in range(self.spec.get("top_buttons", 0))])
        for u,t,w,l,yaw in keys:
            x,y,z = self.point(t, u)
            block("Mode button", (x,y,z+.006), (w,l,.018), .006, self.keys,
                  rotation=(.28, -self.lean, yaw))
        if self.spec.get("status"):
            u,t = self.spec.get("indicator", (self.wheel_x,self.wheel_y-.093))
            x,y,z = self.point(t, u)
            stroke("Status LED", [(x-.008,y,z+.004),(x+.008,y,z+.004)], .0027, self.led)

    def wheel_slot(self, t):
        radius = self.spec.get("slot_length", .088)
        width = self.wheel_width/self.width*.64
        polygon = [(self.wheel_x + math.copysign(abs(math.sin(a))**.42, math.sin(a))*width,
                    t + math.copysign(abs(math.cos(a))**.49, math.cos(a))*radius)
                   for a in (i*math.tau/32 for i in range(32))]
        self.layer("Wheel recess", polygon, self.wheel_housing, .008, self.seam, rounding=False)

    def layer(self, name, polygon, mat, raise_by, rim, rounding=True):
        # Tessellate in plan view, then project short triangles onto the shell.
        # The curved parts cast their own tiny shadows at the assembly gaps.
        flat = [Vector((u,t,0)) for u,t in polygon]
        if rounding:
            flat = rounded(flat)
        triangles = tessellate_polygon([flat])
        points, faces, normals = [], [], []
        cache = {}
        def split(a,b,c,level):
            if level:
                ab,bc,ca = (a+b)/2,(b+c)/2,(c+a)/2
                for tri in ((a,ab,ca),(ab,b,bc),(ca,bc,c),(ab,bc,ca)):
                    split(*tri,level-1)
                return
            face = []
            for v in (a,b,c):
                key = (round(v.x,6),round(v.y,6))
                if key not in cache:
                    cache[key] = len(points)
                    points.append(self.point(v.y,v.x,raise_by))
                    normals.append(self.normal(v.y,v.x))
                face.append(cache[key])
            pa,pb,pc = (points[i] for i in face)
            cross = (pb[0]-pa[0])*(pc[1]-pa[1])-(pb[1]-pa[1])*(pc[0]-pa[0])
            faces.append(tuple(reversed(face)) if cross < 0 else tuple(face))
        # Use a shared subdivision level so adjacent triangles share edges.
        level = 5 if name in ("Palm cover","Click plates") else 4 if len(flat)>12 else 3
        for a,b,c in triangles:
            if isinstance(a, int):
                a,b,c = flat[a],flat[b],flat[c]
            split(a,b,c,level)
        surface = mesh(name,points,faces,mat)
        surface.data.normals_split_custom_set_from_vertices(normals)
        if rim:
            edge = [self.point(a.y+(b.y-a.y)*i/8,a.x+(b.x-a.x)*i/8,raise_by-.001)
                    for a,b in zip(flat,flat[1:]+flat[:1]) for i in range(8)]
            stroke(name+" perimeter",edge+[edge[0]],.00185,rim)

    def normal(self,t,u):
        du = Vector(self.point(t,min(.9999,u+.0004)))-Vector(self.point(t,max(-.9999,u-.0004)))
        dt = Vector(self.point(min(1,t+.0004),u))-Vector(self.point(max(0,t-.0004),u))
        normal = du.cross(dt).normalized()
        return normal if normal.z > 0 else -normal

    def key_label(self,label,u,t):
        bpy.ops.object.text_add(location=self.point(t,u,.016))
        obj = bpy.context.object
        obj.name = "Key "+str(label)
        obj.data.body = str(label)
        obj.data.size = .021
        obj.data.align_x = "CENTER"
        obj.data.align_y = "CENTER"
        obj.data.materials.append(self.led)
        normal = self.normal(t,u)
        up = Vector((0,-1,0))
        right = up.cross(normal).normalized()
        obj.rotation_euler = Matrix((right,normal.cross(right),normal)).transposed().to_euler()

    def layers(self):
        for i, shape in enumerate(self.plans):
            self.layer("Palm cover" if i==2 else "Main button", shape, self.deck, .009, self.seam)
        for shape in self.grips:
            self.layer("Textured grip insert", shape, self.rubber, .006, self.seam)
        if self.spec.get("split_cap"):
            self.layer("Auxiliary leading button",
                       ((-.87,.070),(-.51,.046),(-.57,.132),(-.88,.170),(-.94,.154)),
                       self.keys, .009, self.seam)

    def seams(self):
        depth = self.spec.get("split", .43)
        if not self.plans and self.spec["shape"] != "vertical":
            stroke("Primary split", [self.point(.010+i*(self.wheel_y-.095)/32,self.wheel_x,.002)
                                     for i in range(33)], .0027, self.seam)
        if self.spec.get("continuous_shell"):
            stroke("Center groove", [self.point(self.wheel_y+.07+i*.004,0,.002)
                                     for i in range(41)], .0025, self.seam)
        elif not self.plans and self.spec["shape"] != "vertical":
            for side in (-1,1):
                stroke("Primary key seam", [
                    self.point(depth + .052*side*self.lean - .044*abs(u)
                               + self.spec.get("cut", 0)*u*u, u, .002)
                    for u in (side*i/39 for i in range(2,40))
                ], .004, self.seam)
        if self.spec.get("rail") and not self.grips:
            for side in (-1, 1):
                stroke("Side shell seam", [self.point(.19+i*.60/46, side*.94, .001)
                                           for i in range(47)], .0028, self.seam)

    def thumb_controls(self):
        count = self.spec.get("side_buttons", 2)
        z = self.height_at(.43)*.41+.04
        if count in (6, 7, 12):
            side = 1 if self.spec.get("left_handed") else -1
            base = [(side*u,t) for u,t in ((.99,.330),(.52,.345),(.41,.429),
                                            (.44,.627),(.53,.696),(.93,.681),(.99,.641))]
            self.layer("MMO keypad insert",base,self.rubber,.008,self.seam)
            if count == 12:
                positions = [(side*(.90-col*.125), .387+row*.079-col*.010)
                             for row in range(4) for col in range(3)]
                du,dt = .056,.033
            else:
                positions = [(side*(.71+.145*math.sin(i*math.tau/count)),
                              .503+.105*math.cos(i*math.tau/count)) for i in range(count)]
                du,dt = .064,.030
            for i,(u,t) in enumerate(positions,1):
                polygon = ((u-du,t-dt),(u+du,t-dt-.005),(u+du,t+dt),(u-du,t+dt+.005))
                self.layer("Thumb key",polygon,self.deck,.014,self.seam)
                if self.spec.get("numbered_keys"):
                    self.key_label(i,u,t)
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
        self.layers()
        if self.spec["shape"] == "vertical":
            self.layer("Central spine", ((-.35,.042),(-.26,.014),(-.09,.026),
                                         (-.025,.080),(-.025,.578),(-.12,.626),
                                         (-.28,.604),(-.37,.539)), self.deck,.009,self.seam)
        self.seams()
        self.controls()
        self.thumb_controls()


def rounded(points):
    result = []
    for before,corner,after in zip(points[-1:]+points[:-1],points,points[1:]+points[:1]):
        inset = min((before-corner).length,(after-corner).length,.13)*.25
        start = corner+(before-corner).normalized()*inset
        end = corner+(after-corner).normalized()*inset
        for t in (0,.33,.67,1):
            result.append(start*(1-t)**2+corner*(2*t*(1-t))+end*t*t)
    return result


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
    scene.cycles.samples = 28
    scene.cycles.use_denoising = True
    scene.cycles.max_bounces = 4
    scene.render.film_transparent = True
    scene.render.resolution_x = 484
    scene.render.resolution_y = 620
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "AgX"
    world = scene.world
    world.use_nodes = True
    world.node_tree.nodes.get("Background").inputs[0].default_value = (.35,.37,.39,1)
    world.node_tree.nodes.get("Background").inputs[1].default_value = .26
    bpy.ops.object.camera_add()
    scene.camera = bpy.context.object
    scene.camera.data.type = "ORTHO"
    scene.camera.data.ortho_scale = 1.21
    for name, position, size, energy, color in (
        ("Softbox", (-1.02,-.58,1.86), 1.85, 80, (.95,.97,1)),
        ("Broad fill", (1.12,-.28,1.32), 1.72, 44, (.95,.98,1)),
        ("Heel rim", (.79,1.05,.59), .61, 37, (.90,.95,1)),
        ("Soft edge", (-.88,.72,.67), .91, 14, (.84,.91,1)),
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
            mouse_side = .91 if spec.get("side_buttons", 0) > 5 else .39
            camera.location = (mouse_side*handed, -.46, 2.90)
            back = (camera.location-Vector((0,0,.14))).normalized()
            right = Vector((-.075*handed,-1,0)).cross(back).normalized()
            camera.rotation_euler = Matrix((right, back.cross(right), back)).transposed().to_euler()
            if spec["shape"] == "receiver":
                dongle()
            elif spec["shape"] in ("dock", "dock-pro"):
                dock(spec["shape"] == "dock-pro")
            else:
                Mouse(slug,spec).draw()
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
