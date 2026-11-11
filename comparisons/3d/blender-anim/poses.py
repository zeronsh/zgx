# Blender: a glTF model's joint positions in the world (glTF's axes, y up)
# for each case: (time in seconds, influence of the top track's clip over
# the bottom's). Usage: Blender --background --factory-startup --python
# poses.py -- MODEL BOTTOM TOP OUT.json
import bpy, json, sys
model, bottom, top, out = sys.argv[sys.argv.index("--") + 1:][:4]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=model)
scene = bpy.context.scene
fps = scene.render.fps / scene.render.fps_base
arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
data = arm.animation_data
data.action = None
tracks = {t.name: t for t in data.nla_tracks}
for t in data.nla_tracks:
    t.mute = t.name not in (bottom, top)
    for s in t.strips:
        print("STRIP", t.name, s.name, s.frame_start, s.frame_end, s.blend_type, s.extrapolation)
def curves(action):
    """An action's F-curves (slotted actions keep them in channel bags)."""
    if hasattr(action, "fcurves") and len(action.fcurves):
        return list(action.fcurves)
    found = []
    for layer in getattr(action, "layers", []):
        for strip in layer.strips:
            for bag in strip.channelbags:
                found.extend(bag.fcurves)
    return found

def quaternions(action):
    """Each bone's rotation curves (w, x, y, z), by bone name."""
    bones = {}
    for curve in curves(action):
        path = curve.data_path
        if path.endswith("rotation_quaternion"):
            bone = path.split('"')[1]
            bones.setdefault(bone, [None] * 4)[curve.array_index] = curve
    return {b: c for b, c in bones.items() if all(c)}

def flip(curves, k):
    for curve in curves:
        point = curve.keyframe_points[k]
        point.co[1] = -point.co[1]
        point.handle_left[1] = -point.handle_left[1]
        point.handle_right[1] = -point.handle_right[1]

# `--continuous`: glTF turns along the shorter arc between keys and blends
# so; Blender interpolates and mixes a quaternion's components apart. Each
# key flipped to its neighbour's side, and the top clip's to the bottom's,
# the two meet the same turns.
if "--continuous" in sys.argv:
    bottom_q = quaternions(bpy.data.actions[bottom])
    for name in (bottom, top):
        for bone, four in quaternions(bpy.data.actions[name]).items():
            points = len(four[0].keyframe_points)
            value = lambda k: [c.keyframe_points[k].co[1] for c in four]
            for k in range(1, points):
                if sum(a * b for a, b in zip(value(k - 1), value(k))) < 0:
                    flip(four, k)
            if name == top and bone in bottom_q:
                under = [c.evaluate(c.keyframe_points[0].co[0]) for c in bottom_q[bone]]
                if sum(a * b for a, b in zip(under, value(0))) < 0:
                    for k in range(points):
                        flip(four, k)
            for c in four:
                c.update()
    print("CONTINUOUS")

# Bottom below top, as the importer laid them out or not.
names = [t.name for t in data.nla_tracks]
assert names.index(bottom) < names.index(top), names
strip = tracks[top].strips[0]
assert tracks[bottom].strips[0].frame_start == strip.frame_start
strip.use_animated_influence = True
cases = []
for t in [0.0, 0.1, 0.25, 0.4, 0.55, 0.7]:
    for w in [0.0, 0.25, 0.5, 0.75, 1.0]:
        cases.append((t, w))
result = []
for t, w in cases:
    # Animated influence is keyed (enabling it keys it): the key, not the
    # property, says what a frame blends.
    strip.influence = w
    for curve in strip.fcurves:
        if curve.data_path == "influence":
            for key in curve.keyframe_points:
                key.co[1] = w
                key.handle_left[1] = w
                key.handle_right[1] = w
            curve.update()
    # A strip plays its action from its start (the importer's: frame 1).
    frame = strip.frame_start + t * fps
    scene.frame_set(int(frame), subframe=frame - int(frame))
    bpy.context.view_layer.update()
    joints = {}
    for bone in arm.pose.bones:
        p = arm.matrix_world @ bone.head
        joints[bone.name] = [p.x, p.z, -p.y]
    result.append({"time": t, "weight": w, "joints": joints})
json.dump({"fps": fps, "cases": result}, open(out, "w"))
print("WROTE", len(result))
