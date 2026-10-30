"""A MapCraft plugin written in Python: a staircase generator.

Build it with componentize-py from this folder:
    componentize-py -d ../../wit -w plugin componentize app -o stairs.wasm
"""

import wit_world
from componentize_py_types import Err
from wit_world.imports import host
from wit_world.imports.types import (
    Brush_Hull,
    CommandSpec,
    Hull,
    IntegerRange,
    Manifest,
    NumberRange,
    Output,
    ParamKind_Integer,
    ParamKind_Material,
    ParamKind_Number,
    ParamSpec,
    Value_Integer,
    Value_Number,
    Value_Text,
    Vec3,
)


def number(id, label, low, high, initial):
    return ParamSpec(id, label, "", ParamKind_Number(NumberRange(low, high, 1.0)), Value_Number(initial))


STAIRS = CommandSpec(
    id="stairs",
    label="Stairs",
    menu="Examples/Stairs",
    params=[
        ParamSpec("steps", "Steps", "How many steps", ParamKind_Integer(IntegerRange(1, 256)), Value_Integer(8)),
        number("rise", "Step height", 1.0, 512.0, 8.0),
        number("run", "Step depth", 1.0, 512.0, 16.0),
        number("width", "Width", 1.0, 4096.0, 64.0),
        ParamSpec("material", "Material", "Empty uses the current material", ParamKind_Material(), Value_Text("")),
    ],
    generator=True,
)


def box(x0, y0, z0, x1, y1, z1, material):
    points = [Vec3(x, y, z) for x in (x0, x1) for y in (y0, y1) for z in (z0, z1)]
    return Brush_Hull(Hull(points, material, []))


class WitWorld(wit_world.WitWorld):
    def manifest(self):
        return Manifest("stairs", "Stairs (Python example)", "0.1.0", [STAIRS], False, [], [], False, [])

    def run_command(self, id, params, context):
        if id != "stairs":
            raise Err(f"unknown command {id}")
        p = {param.id: param.value.value for param in params}
        steps, rise, run, width = int(p["steps"]), p["rise"], p["run"], p["width"]
        host.log(f"stairs: {steps} steps, {steps * rise:g} units high")
        brushes = [box(i * run, -width / 2, 0.0, (i + 1) * run, width / 2, (i + 1) * rise, p["material"]) for i in range(steps)]
        return Output(brushes, [], [], [])

    def list_materials(self):
        return []

    def import_file(self, importer, name, data, context):
        raise Err("this plugin imports no files")

    def export_file(self, exporter, map, context):
        raise Err("this plugin exports no files")

    def validate(self, map):
        return []

    def load_materials(self, names):
        return [None for _ in names]

    def load_model(self, path):
        raise Err(f"this plugin reads no models: {path}")

    def list_models(self):
        return []

    def compile_issues(self, map, log):
        return []
