"""Differential test of kiln-blocks against the vanilla 26.3 server.

Builds block scenarios (shapes, pop-offs, fluids, redstone) in a flat vanilla world with the
game frozen, steps it tick by tick with `/tick step`, saves snapshots, then replays the same
commands on kiln-blocks' TestLevel (`cargo run --example vanilla_diff`) and compares every
block and pending scheduled tick in each scenario's box.

usage: python tools/blocks_diff.py [--port 25592] [--out DIR] [--only NAME,...] [--skip-vanilla]
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK", ROOT / "work"))
SURFACE = -60  # flat world: bedrock at -64, stone -63..-61
LAYERS = ["minecraft:bedrock", "minecraft:stone", "minecraft:stone", "minecraft:stone"]
SNAPSHOTS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 15, 20, 25, 30, 40, 50, 60, 80, 100, 130, 160, 200, 250, 300]
GRID = 24  # scenario spacing in blocks
COLUMNS = 8


class Scenario:
    def __init__(self, name, size=(16, 8, 16)):
        self.name = name
        self.size = size
        self.setup = []  # (dx, dy, dz, command template)
        self.events = {}  # tick -> [commands]

    def at(self, tick, cmd):
        self.events.setdefault(tick, []).append(cmd)
        return self

    def cmd(self, cmd):
        self.setup.append(cmd)
        return self


SCENARIOS = []


def scenario(name, size=(16, 8, 16)):
    def wrap(fn):
        SCENARIOS.append((name, size, fn))
        return fn
    return wrap


# Scenario bodies get `b(x, y, z, block, mode="")` for /setblock, `f(...)` for /fill and
# `at(tick, fn)` to run commands later. Coordinates are relative to the scenario origin on
# the surface.

@scenario("fence_connections")
def _(b, f, at):
    f(1, 0, 1, 6, 0, 1, "minecraft:oak_fence")
    b(7, 0, 1, "minecraft:stone")
    b(1, 0, 2, "minecraft:nether_brick_fence")
    b(3, 0, 2, "minecraft:glass_pane")
    b(5, 0, 2, "minecraft:oak_fence_gate[facing=north]")
    b(6, 0, 2, "minecraft:oak_fence_gate[facing=east]")
    b(2, 0, 3, "minecraft:oak_leaves")
    b(2, 0, 4, "minecraft:oak_fence")
    b(1, 0, 4, "minecraft:pumpkin")
    b(3, 0, 4, "minecraft:oak_stairs[facing=west]")
    at(2, lambda b, f: b(4, 0, 1, "minecraft:air"))
    at(5, lambda b, f: b(7, 0, 1, "minecraft:air"))


@scenario("panes_and_bars")
def _(b, f, at):
    f(1, 0, 1, 6, 0, 1, "minecraft:glass_pane")
    f(1, 0, 3, 6, 0, 3, "minecraft:iron_bars")
    b(3, 0, 2, "minecraft:red_stained_glass_pane")
    b(7, 0, 1, "minecraft:cobblestone_wall")
    b(7, 0, 3, "minecraft:stone_slab")
    b(0, 0, 3, "minecraft:glowstone")
    at(3, lambda b, f: b(3, 0, 2, "minecraft:air"))


@scenario("walls")
def _(b, f, at):
    f(1, 0, 1, 6, 0, 1, "minecraft:cobblestone_wall")
    b(2, 1, 1, "minecraft:stone")
    b(4, 1, 1, "minecraft:torch")
    b(5, 1, 1, "minecraft:stone_slab")
    b(3, 0, 2, "minecraft:cobblestone_wall")
    b(3, 0, 3, "minecraft:stone_bricks")
    b(6, 0, 2, "minecraft:iron_bars")
    b(1, 0, 5, "minecraft:andesite_wall")
    b(1, 1, 5, "minecraft:andesite_wall")
    b(2, 0, 5, "minecraft:oak_fence_gate[facing=north]")
    at(4, lambda b, f: b(2, 1, 1, "minecraft:air"))
    at(6, lambda b, f: b(5, 1, 1, "minecraft:stone_slab[type=top]"))


@scenario("stairs_shapes")
def _(b, f, at):
    b(1, 0, 1, "minecraft:oak_stairs[facing=north]")
    b(2, 0, 1, "minecraft:oak_stairs[facing=west]")
    b(4, 0, 1, "minecraft:stone_stairs[facing=east]")
    b(4, 0, 2, "minecraft:stone_stairs[facing=north]")
    b(6, 0, 1, "minecraft:oak_stairs[facing=south,half=top]")
    b(7, 0, 1, "minecraft:oak_stairs[facing=east,half=top]")
    b(1, 0, 4, "minecraft:brick_stairs[facing=south]")
    b(2, 0, 4, "minecraft:brick_stairs[facing=south]")
    b(2, 0, 5, "minecraft:brick_stairs[facing=east]")
    at(3, lambda b, f: b(2, 0, 1, "minecraft:oak_stairs[facing=east]"))


@scenario("snowy_grass")
def _(b, f, at):
    f(1, -1, 1, 5, -1, 3, "minecraft:grass_block")
    b(2, 0, 2, "minecraft:snow")
    b(4, 0, 2, "minecraft:snow_block")
    b(3, -1, 1, "minecraft:podzol")
    b(3, 0, 1, "minecraft:snow")
    at(2, lambda b, f: b(2, 0, 2, "minecraft:air"))


@scenario("wire_shapes")
def _(b, f, at):
    f(1, 0, 1, 5, 0, 1, "minecraft:redstone_wire")
    b(3, 0, 3, "minecraft:redstone_wire")
    b(3, 0, 5, "minecraft:redstone_wire")
    b(4, 0, 5, "minecraft:redstone_wire")
    b(6, 0, 3, "minecraft:stone")
    b(6, 1, 3, "minecraft:redstone_wire")
    b(5, 0, 3, "minecraft:redstone_wire")
    b(8, 0, 1, "minecraft:repeater[facing=west]")
    b(7, 0, 1, "minecraft:redstone_wire")
    b(9, 0, 1, "minecraft:redstone_wire")
    b(8, 0, 3, "minecraft:redstone_torch")
    b(8, 0, 4, "minecraft:redstone_wire")
    b(1, 0, 7, "minecraft:redstone_wire")
    b(1, 0, 8, "minecraft:lever[face=floor]")
    at(3, lambda b, f: b(6, 1, 3, "minecraft:air"))


@scenario("doors_and_plants")
def _(b, f, at):
    b(1, 0, 1, "minecraft:oak_door[half=lower]")
    b(1, 1, 1, "minecraft:oak_door[half=upper]")
    b(3, 0, 1, "minecraft:iron_door[half=lower,facing=east]")
    b(3, 1, 1, "minecraft:iron_door[half=upper,facing=east]")
    f(5, -1, 1, 8, -1, 2, "minecraft:grass_block")
    b(5, 0, 1, "minecraft:sunflower[half=lower]")
    b(5, 1, 1, "minecraft:sunflower[half=upper]")
    b(6, 0, 1, "minecraft:tall_grass[half=lower]")
    b(6, 1, 1, "minecraft:tall_grass[half=upper]")
    b(7, 0, 1, "minecraft:poppy")
    b(8, 0, 1, "minecraft:oak_sapling")
    b(7, 0, 2, "minecraft:short_grass")
    at(2, lambda b, f: b(1, 0, 1, "minecraft:air"))
    at(4, lambda b, f: b(3, 1, 1, "minecraft:air"))
    at(6, lambda b, f: b(5, 0, 1, "minecraft:air"))
    at(8, lambda b, f: b(6, 1, 1, "minecraft:air"))
    at(10, lambda b, f: b(7, -1, 1, "minecraft:stone"))
    at(12, lambda b, f: b(8, -1, 1, "minecraft:sand"))


@scenario("pop_offs")
def _(b, f, at):
    b(1, 0, 1, "minecraft:stone")
    b(1, 1, 1, "minecraft:torch")
    b(2, 0, 1, "minecraft:wall_torch[facing=east]")
    b(1, 0, 3, "minecraft:stone")
    b(1, 1, 3, "minecraft:white_carpet")
    b(2, 0, 3, "minecraft:lever[face=wall,facing=east]")
    b(1, 0, 4, "minecraft:stone_button[face=wall,facing=south]")
    b(1, 0, 5, "minecraft:stone")
    b(1, 1, 5, "minecraft:repeater")
    b(1, 0, 6, "minecraft:ladder[facing=south]")
    b(4, 0, 1, "minecraft:stone")
    b(4, 1, 1, "minecraft:redstone_wire")
    b(4, 0, 2, "minecraft:redstone_wall_torch[facing=south]")
    at(2, lambda b, f: b(1, 0, 1, "minecraft:air"))
    at(3, lambda b, f: b(1, 0, 3, "minecraft:air"))
    at(4, lambda b, f: b(1, 0, 5, "minecraft:air"))
    at(5, lambda b, f: b(4, 0, 1, "minecraft:glass"))


def water_pool(b, f, at, block="minecraft:water"):
    b(8, 0, 8, block)


@scenario("water_flat", (24, 6, 24))
def _(b, f, at):
    b(11, 0, 11, "minecraft:water")


@scenario("water_steps", (24, 10, 24))
def _(b, f, at):
    f(2, 0, 2, 20, 4, 20, "minecraft:stone")
    f(5, 4, 5, 17, 4, 17, "minecraft:air")
    f(8, 3, 8, 14, 4, 14, "minecraft:air")
    b(11, 4, 11, "minecraft:water")
    b(3, 4, 11, "minecraft:water")


@scenario("water_hole", (24, 6, 24))
def _(b, f, at):
    f(1, 0, 1, 22, 0, 22, "minecraft:stone")
    b(15, 0, 11, "minecraft:air")
    b(11, 0, 17, "minecraft:air")
    b(11, 1, 11, "minecraft:water")


@scenario("water_sources", (16, 6, 16))
def _(b, f, at):
    b(4, 0, 4, "minecraft:water")
    b(6, 0, 4, "minecraft:water")
    b(4, 0, 9, "minecraft:water")
    b(4, 0, 11, "minecraft:water")
    b(8, 0, 12, "minecraft:water")
    at(40, lambda b, f: b(4, 0, 4, "minecraft:air"))


@scenario("water_washes", (16, 6, 16))
def _(b, f, at):
    f(3, 0, 3, 12, 0, 12, "minecraft:torch")
    b(8, 0, 8, "minecraft:water")
    b(4, 0, 12, "minecraft:redstone_wire")
    b(5, 0, 12, "minecraft:rail")


@scenario("waterlogged", (16, 6, 16))
def _(b, f, at):
    b(3, 0, 3, "minecraft:oak_stairs[waterlogged=true]")
    b(8, 0, 3, "minecraft:stone_slab[waterlogged=true]")
    b(12, 0, 8, "minecraft:oak_fence[waterlogged=true]")
    b(3, 0, 10, "minecraft:water")
    b(4, 0, 10, "minecraft:oak_slab")
    b(3, 0, 11, "minecraft:oak_stairs[facing=north]")


@scenario("lava_flat", (16, 6, 16))
def _(b, f, at):
    b(8, 0, 8, "minecraft:lava")


@scenario("lava_water", (20, 8, 20))
def _(b, f, at):
    b(4, 0, 4, "minecraft:lava")
    b(8, 0, 4, "minecraft:water")
    b(4, 0, 12, "minecraft:water")
    b(4, 2, 12, "minecraft:lava")
    b(12, 0, 12, "minecraft:lava")
    b(12, 3, 12, "minecraft:water")
    b(15, 0, 4, "minecraft:lava")
    b(16, 0, 4, "minecraft:water")


@scenario("wire_line", (20, 6, 8))
def _(b, f, at):
    f(1, 0, 2, 18, 0, 2, "minecraft:redstone_wire")
    b(0, 0, 2, "minecraft:redstone_block")
    f(1, 0, 5, 6, 0, 5, "minecraft:redstone_wire")
    b(7, 0, 5, "minecraft:stone")
    b(7, 1, 5, "minecraft:redstone_wire")
    b(8, 0, 5, "minecraft:redstone_wire")
    at(3, lambda b, f: b(0, 0, 5, "minecraft:redstone_block"))
    at(8, lambda b, f: b(0, 0, 2, "minecraft:air"))


@scenario("torch_tower", (8, 14, 8))
def _(b, f, at):
    for i in range(5):
        b(3, 2 * i, 3, "minecraft:stone")
        b(3, 2 * i + 1, 3, "minecraft:redstone_torch")
    at(4, lambda b, f: b(2, 0, 3, "minecraft:redstone_block"))
    at(30, lambda b, f: b(2, 0, 3, "minecraft:air"))


@scenario("repeater_chain", (20, 6, 8))
def _(b, f, at):
    b(0, 0, 2, "minecraft:lever[face=floor,powered=false]")
    for i, delay in enumerate([1, 2, 3, 4, 1, 4]):
        b(1 + 2 * i, 0, 2, f"minecraft:repeater[facing=west,delay={delay}]")
        b(2 + 2 * i, 0, 2, "minecraft:redstone_wire")
    b(14, 0, 2, "minecraft:redstone_lamp")
    at(2, lambda b, f: b(0, 0, 2, "minecraft:redstone_block"))
    at(12, lambda b, f: b(0, 0, 2, "minecraft:air"))
    at(14, lambda b, f: b(0, 0, 2, "minecraft:redstone_block"))
    at(40, lambda b, f: b(0, 0, 2, "minecraft:air"))


@scenario("repeater_lock", (12, 6, 12))
def _(b, f, at):
    b(2, 0, 5, "minecraft:repeater[facing=west]")
    b(3, 0, 5, "minecraft:redstone_wire")
    b(1, 0, 5, "minecraft:redstone_block")
    b(2, 0, 4, "minecraft:repeater[facing=south]")
    b(2, 0, 3, "minecraft:air")
    at(3, lambda b, f: b(2, 0, 3, "minecraft:redstone_block"))
    at(10, lambda b, f: b(1, 0, 5, "minecraft:air"))
    at(20, lambda b, f: b(2, 0, 3, "minecraft:air"))


@scenario("torch_clock_burnout", (12, 6, 12))
def _(b, f, at):
    # A wall torch whose own output loops back into its block: it toggles every 2 ticks
    # until eight toggles within 60 ticks burn it out.
    b(3, 0, 3, "minecraft:stone")
    b(4, 0, 3, "minecraft:redstone_wall_torch[facing=east]")
    for x, z in [(5, 3), (5, 4), (5, 5), (5, 6), (4, 6), (3, 6), (3, 5), (3, 4)]:
        b(x, 0, z, "minecraft:redstone_wire")


@scenario("leaves_distance", (16, 10, 16))
def _(b, f, at):
    f(4, 0, 4, 4, 4, 4, "minecraft:oak_log")
    f(2, 3, 2, 9, 5, 9, "minecraft:oak_leaves[persistent=false,distance=7]", "keep")
    b(8, 5, 8, "minecraft:oak_log")
    at(10, lambda b, f: f(4, 0, 4, 4, 4, 4, "minecraft:air"))
    at(30, lambda b, f: b(8, 5, 8, "minecraft:air"))


@scenario("trapdoors_gates", (16, 6, 8))
def _(b, f, at):
    b(1, 0, 2, "minecraft:oak_trapdoor[facing=north]")
    b(3, 0, 2, "minecraft:iron_trapdoor[facing=east,half=top]")
    b(5, 0, 2, "minecraft:oak_fence_gate[facing=north]")
    b(6, 0, 2, "minecraft:cobblestone_wall")
    b(4, 0, 2, "minecraft:cobblestone_wall")
    b(8, 0, 2, "minecraft:birch_fence_gate[facing=east]")
    at(2, lambda b, f: b(2, 0, 2, "minecraft:redstone_block"))
    at(4, lambda b, f: b(5, 0, 3, "minecraft:redstone_block"))
    at(6, lambda b, f: b(8, 0, 1, "minecraft:redstone_torch"))
    at(12, lambda b, f: b(2, 0, 2, "minecraft:air"))
    at(14, lambda b, f: b(5, 0, 3, "minecraft:air"))
    at(16, lambda b, f: b(6, 0, 2, "minecraft:air"))


@scenario("lava_steps", (20, 10, 20))
def _(b, f, at):
    f(2, 0, 2, 16, 3, 16, "minecraft:stone")
    f(4, 3, 4, 14, 3, 14, "minecraft:air")
    f(6, 2, 6, 12, 3, 12, "minecraft:air")
    b(9, 3, 9, "minecraft:lava")
    b(3, 3, 9, "minecraft:lava")


@scenario("water_channel", (24, 6, 8))
def _(b, f, at):
    f(0, 0, 1, 23, 1, 1, "minecraft:stone")
    f(0, 0, 3, 23, 1, 3, "minecraft:stone")
    b(1, 0, 2, "minecraft:water")
    b(12, -1, 2, "minecraft:air")
    at(60, lambda b, f: b(1, 0, 2, "minecraft:stone"))


@scenario("wire_grid", (12, 6, 12))
def _(b, f, at):
    f(2, 0, 2, 8, 0, 8, "minecraft:redstone_wire")
    b(5, 0, 5, "minecraft:stone")
    b(5, 1, 5, "minecraft:redstone_lamp")
    b(1, 0, 5, "minecraft:repeater[facing=east]")
    b(9, 0, 3, "minecraft:repeater[facing=west]")
    b(5, 0, 9, "minecraft:redstone_torch")
    at(2, lambda b, f: b(0, 0, 5, "minecraft:redstone_block"))
    at(9, lambda b, f: b(10, 0, 3, "minecraft:redstone_block"))
    at(20, lambda b, f: b(0, 0, 5, "minecraft:air"))
    at(25, lambda b, f: b(5, 0, 9, "minecraft:air"))


@scenario("button_lever", (12, 6, 8))
def _(b, f, at):
    b(2, 0, 2, "minecraft:stone")
    b(2, 1, 2, "minecraft:oak_button[face=floor,powered=true]")
    b(3, 0, 2, "minecraft:redstone_lamp")
    b(5, 0, 2, "minecraft:stone")
    b(6, 0, 2, "minecraft:lever[face=wall,facing=east,powered=true]")
    b(4, 0, 2, "minecraft:redstone_wire")
    b(7, 0, 3, "minecraft:redstone_lamp")
    at(5, lambda b, f: b(6, 0, 2, "minecraft:air"))
    at(8, lambda b, f: b(2, 1, 2, "minecraft:air"))


@scenario("repeater_clock", (12, 6, 12))
def _(b, f, at):
    b(3, 0, 3, "minecraft:redstone_wire")
    b(4, 0, 3, "minecraft:repeater[facing=west,delay=2]")
    b(5, 0, 3, "minecraft:redstone_wire")
    b(5, 0, 4, "minecraft:redstone_wire")
    b(4, 0, 4, "minecraft:repeater[facing=east,delay=3]")
    b(3, 0, 4, "minecraft:redstone_wire")
    b(2, 0, 3, "minecraft:redstone_lamp")
    at(2, lambda b, f: b(3, 0, 2, "minecraft:redstone_block"))
    at(3, lambda b, f: b(3, 0, 2, "minecraft:air"))


@scenario("lamps_doors", (16, 6, 8))
def _(b, f, at):
    b(1, 0, 2, "minecraft:redstone_lamp")
    b(2, 0, 2, "minecraft:lever[face=floor,powered=true]")
    b(4, 0, 2, "minecraft:redstone_lamp")
    b(6, 0, 2, "minecraft:oak_door[half=lower]")
    b(6, 1, 2, "minecraft:oak_door[half=upper]")
    b(9, 0, 2, "minecraft:redstone_lamp")
    b(10, 0, 2, "minecraft:redstone_wire")
    at(3, lambda b, f: b(5, 0, 2, "minecraft:redstone_block"))
    at(6, lambda b, f: b(4, 1, 2, "minecraft:redstone_block"))
    at(9, lambda b, f: b(11, 0, 2, "minecraft:redstone_block"))
    at(20, lambda b, f: b(5, 0, 2, "minecraft:air"))
    at(22, lambda b, f: b(4, 1, 2, "minecraft:air"))
    at(24, lambda b, f: b(11, 0, 2, "minecraft:air"))


@scenario("wire_slopes", (16, 8, 8))
def _(b, f, at):
    for i in range(5):
        b(2 + i, i, 2, "minecraft:stone")
        b(2 + i, i + 1, 2, "minecraft:redstone_wire")
    b(1, 0, 2, "minecraft:redstone_wire")
    b(8, 5, 2, "minecraft:redstone_wire")
    b(8, 4, 2, "minecraft:stone")
    at(2, lambda b, f: b(0, 0, 2, "minecraft:redstone_block"))
    at(12, lambda b, f: b(0, 0, 2, "minecraft:air"))


@scenario("observers", (18, 6, 8))
def _(b, f, at):
    b(2, 0, 2, "minecraft:observer[facing=west]")
    b(3, 0, 2, "minecraft:redstone_lamp")
    b(6, 0, 2, "minecraft:observer[facing=north]")
    b(6, 0, 3, "minecraft:redstone_wire")
    b(6, 0, 4, "minecraft:redstone_wire")
    b(9, 0, 2, "minecraft:observer[facing=up]")
    b(9, 0, 3, "minecraft:redstone_lamp")
    b(12, 0, 2, "minecraft:observer[facing=east]")
    b(13, 0, 2, "minecraft:observer[facing=west]")
    b(12, 0, 3, "minecraft:redstone_wire")
    at(3, lambda b, f: b(1, 0, 2, "minecraft:stone"))
    at(8, lambda b, f: b(6, 0, 1, "minecraft:oak_planks"))
    at(12, lambda b, f: b(9, 1, 2, "minecraft:redstone_block"))
    at(20, lambda b, f: b(1, 0, 2, "minecraft:air"))


@scenario("note_blocks", (12, 6, 8))
def _(b, f, at):
    b(2, 0, 2, "minecraft:note_block")
    b(4, -1, 2, "minecraft:gold_block")
    b(4, 0, 2, "minecraft:note_block")
    b(6, 0, 2, "minecraft:note_block[note=5]")
    b(6, 1, 2, "minecraft:zombie_head")
    b(5, 0, 2, "minecraft:redstone_wire")
    at(3, lambda b, f: b(3, 0, 2, "minecraft:redstone_block"))
    at(6, lambda b, f: b(4, -1, 2, "minecraft:clay"))
    at(8, lambda b, f: b(3, 0, 2, "minecraft:air"))
    at(10, lambda b, f: b(5, 0, 1, "minecraft:redstone_torch"))


@scenario("tnt_in_water", (12, 8, 12))
def _(b, f, at):
    f(1, 0, 1, 9, 5, 9, "minecraft:glass", "hollow")
    f(2, 1, 2, 8, 3, 8, "minecraft:water")
    b(5, 1, 5, "minecraft:tnt")
    b(5, 2, 5, "minecraft:tnt")
    at(3, lambda b, f: b(6, 1, 5, "minecraft:redstone_block"))


@scenario("pressure_plates", (12, 6, 8))
def _(b, f, at):
    b(2, 0, 2, "minecraft:stone_pressure_plate[powered=true]")
    b(3, 0, 2, "minecraft:redstone_lamp")
    b(5, 0, 2, "minecraft:heavy_weighted_pressure_plate[power=7]")
    b(5, 0, 3, "minecraft:redstone_wire")
    b(8, 0, 2, "minecraft:oak_pressure_plate")
    at(4, lambda b, f: b(2, -1, 2, "minecraft:glass"))
    at(6, lambda b, f: b(8, -1, 2, "minecraft:air"))


@scenario("rails", (16, 6, 16))
def _(b, f, at):
    f(1, 0, 1, 6, 0, 1, "minecraft:rail")
    f(1, 0, 2, 1, 0, 5, "minecraft:rail")
    b(3, 0, 3, "minecraft:rail")
    b(4, 0, 3, "minecraft:rail")
    b(3, 0, 4, "minecraft:rail")
    b(8, 0, 2, "minecraft:stone")
    b(8, 1, 2, "minecraft:rail")
    b(7, 0, 2, "minecraft:rail")
    b(6, 0, 2, "minecraft:rail")
    b(10, 0, 5, "minecraft:rail")
    b(10, 0, 6, "minecraft:rail")
    b(11, 0, 5, "minecraft:rail")
    b(9, 0, 5, "minecraft:rail")
    at(3, lambda b, f: b(8, 0, 2, "minecraft:air"))
    at(5, lambda b, f: b(1, -1, 3, "minecraft:air"))
    at(7, lambda b, f: b(10, 0, 4, "minecraft:redstone_block"))


@scenario("powered_rails", (20, 6, 8))
def _(b, f, at):
    f(1, 0, 2, 12, 0, 2, "minecraft:powered_rail")
    f(1, 0, 4, 6, 0, 4, "minecraft:activator_rail")
    b(14, 0, 2, "minecraft:stone")
    b(14, 1, 2, "minecraft:powered_rail")
    b(13, 0, 2, "minecraft:powered_rail")
    b(1, 0, 5, "minecraft:detector_rail")
    at(2, lambda b, f: b(0, 0, 2, "minecraft:redstone_block"))
    at(4, lambda b, f: b(0, 0, 4, "minecraft:redstone_torch"))
    at(10, lambda b, f: b(0, 0, 2, "minecraft:air"))
    at(12, lambda b, f: b(15, 1, 2, "minecraft:redstone_block"))


@scenario("comparators", (16, 6, 12))
def _(b, f, at):
    b(1, 0, 2, "minecraft:water_cauldron[level=2]")
    b(2, 0, 2, "minecraft:comparator[facing=west]")
    f(3, 0, 2, 6, 0, 2, "minecraft:redstone_wire")
    b(1, 0, 5, "minecraft:cake[bites=3]")
    b(2, 0, 5, "minecraft:stone")
    b(3, 0, 5, "minecraft:comparator[facing=west]")
    f(4, 0, 5, 7, 0, 5, "minecraft:redstone_wire")
    b(1, 0, 8, "minecraft:composter[level=6]")
    b(2, 0, 8, "minecraft:comparator[facing=west,mode=subtract]")
    b(2, 0, 9, "minecraft:redstone_wire")
    b(3, 0, 9, "minecraft:comparator[facing=east]")
    b(4, 0, 9, "minecraft:water_cauldron[level=1]")
    f(3, 0, 8, 6, 0, 8, "minecraft:redstone_wire")
    b(10, 0, 2, "minecraft:comparator[facing=west,mode=subtract]")
    b(11, 0, 2, "minecraft:redstone_wire")
    b(11, 0, 3, "minecraft:redstone_wire")
    b(10, 0, 3, "minecraft:redstone_wire")
    at(2, lambda b, f: b(9, 0, 2, "minecraft:redstone_block"))
    at(3, lambda b, f: b(1, 0, 2, "minecraft:water_cauldron[level=3]"))
    at(6, lambda b, f: b(1, 0, 5, "minecraft:cake[bites=0]"))
    at(9, lambda b, f: b(4, 0, 9, "minecraft:water_cauldron[level=3]"))
    at(12, lambda b, f: b(1, 0, 2, "minecraft:end_portal_frame[eye=true]"))
    at(15, lambda b, f: b(1, 0, 8, "minecraft:air"))
    at(40, lambda b, f: b(9, 0, 2, "minecraft:air"))



# Pistons: redstone blocks behind them at t2, gone at t12 unless noted.
def power(b, at, spots, on=2, off=12):
    for x, y, z in spots:
        at(on, lambda b, f, x=x, y=y, z=z: b(x, y, z, "minecraft:redstone_block"))
        if off is not None:
            at(off, lambda b, f, x=x, y=y, z=z: b(x, y, z, "minecraft:air"))


@scenario("pistons_push", (16, 6, 12))
def _(b, f, at):
    b(1, 0, 1, "minecraft:piston[facing=east]")
    f(2, 0, 1, 4, 0, 1, "minecraft:stone")
    b(1, 0, 3, "minecraft:sticky_piston[facing=east]")
    b(2, 0, 3, "minecraft:oak_planks")
    b(1, 0, 5, "minecraft:piston[facing=east]")
    f(2, 0, 5, 13, 0, 5, "minecraft:cobblestone")
    b(1, 0, 7, "minecraft:sticky_piston[facing=east]")
    f(2, 0, 7, 14, 0, 7, "minecraft:cobblestone")
    b(1, 0, 9, "minecraft:sticky_piston[facing=up]")
    f(1, 1, 9, 1, 2, 9, "minecraft:stone")
    b(10, 0, 9, "minecraft:piston[facing=west]")
    b(9, 0, 9, "minecraft:iron_block")
    power(b, at, [(0, 0, 1), (0, 0, 3), (0, 0, 5), (0, 0, 7), (0, 0, 9), (11, 0, 9)])


@scenario("piston_reactions", (16, 6, 16))
def _(b, f, at):
    b(1, 0, 1, "minecraft:piston[facing=east]")
    b(2, 0, 1, "minecraft:stone")
    b(3, 0, 1, "minecraft:torch")
    b(1, 0, 3, "minecraft:piston[facing=east]")
    b(2, 0, 3, "minecraft:dandelion")
    b(1, 0, 5, "minecraft:piston[facing=east]")
    b(2, 0, 5, "minecraft:obsidian")
    b(1, 0, 7, "minecraft:sticky_piston[facing=east]")
    b(2, 0, 7, "minecraft:white_glazed_terracotta")
    b(1, 0, 9, "minecraft:sticky_piston[facing=east]")
    b(2, 0, 9, "minecraft:piston[facing=north]")
    b(1, 0, 11, "minecraft:sticky_piston[facing=east]")
    b(2, 0, 11, "minecraft:furnace")
    b(1, 0, 13, "minecraft:piston[facing=east]")
    b(2, 0, 13, "minecraft:stone")
    b(3, 0, 13, "minecraft:bedrock")
    b(8, 0, 1, "minecraft:piston[facing=east]")
    b(9, 0, 1, "minecraft:stone")
    f(10, 0, 1, 12, 0, 1, "minecraft:redstone_wire")
    b(8, 0, 4, "minecraft:piston[facing=east]")
    b(9, 0, 4, "minecraft:oak_slab[type=top,waterlogged=true]")
    power(b, at, [(0, 0, z) for z in (1, 3, 5, 7, 9, 11, 13)] + [(7, 0, 1), (7, 0, 4)])


@scenario("slime_honey", (16, 10, 16))
def _(b, f, at):
    b(1, 3, 1, "minecraft:sticky_piston[facing=east]")
    b(2, 3, 1, "minecraft:slime_block")
    b(2, 3, 2, "minecraft:stone")
    b(2, 4, 1, "minecraft:honey_block")
    b(2, 5, 1, "minecraft:stone")
    b(1, 3, 5, "minecraft:sticky_piston[facing=east]")
    f(2, 3, 5, 3, 3, 5, "minecraft:honey_block")
    b(2, 4, 5, "minecraft:oak_planks")
    b(3, 2, 5, "minecraft:glass")
    b(10, 0, 5, "minecraft:sticky_piston[facing=up]")
    b(10, 1, 5, "minecraft:slime_block")
    b(11, 1, 5, "minecraft:stone")
    b(10, 1, 6, "minecraft:slime_block")
    b(10, 1, 7, "minecraft:cobblestone")
    b(1, 3, 9, "minecraft:piston[facing=east]")
    f(2, 3, 9, 4, 5, 11, "minecraft:slime_block")
    b(1, 3, 13, "minecraft:sticky_piston[facing=east]")
    b(2, 3, 13, "minecraft:slime_block")
    b(3, 3, 13, "minecraft:honey_block")
    b(3, 3, 14, "minecraft:stone")
    b(2, 3, 12, "minecraft:stone")
    power(b, at, [(0, 3, 1), (0, 3, 5), (9, 0, 5), (0, 3, 9), (0, 3, 13)])


@scenario("piston_qc_bud", (12, 6, 10))
def _(b, f, at):
    b(1, 0, 1, "minecraft:piston[facing=east]")
    b(2, 0, 1, "minecraft:stone")
    at(2, lambda b, f: b(0, 1, 1, "minecraft:redstone_block"))
    at(6, lambda b, f: b(1, 0, 0, "minecraft:stone"))
    b(1, 0, 4, "minecraft:piston[facing=east]")
    b(5, 0, 4, "minecraft:piston[facing=up]")
    b(8, 0, 4, "minecraft:sticky_piston[facing=north]")
    b(8, 0, 3, "minecraft:stone")
    power(b, at, [(1, 1, 4), (5, 1, 4), (8, 2, 4)])
    at(20, lambda b, f: b(0, 1, 1, "minecraft:air"))
    at(24, lambda b, f: b(2, 1, 1, "minecraft:stone"))


@scenario("piston_pulses", (12, 6, 10))
def _(b, f, at):
    for z, off in ((1, 3), (3, 4), (5, 5)):
        b(1, 0, z, "minecraft:sticky_piston[facing=east]")
        b(2, 0, z, "minecraft:stone")
        power(b, at, [(0, 0, z)], on=2, off=off)
    b(1, 0, 7, "minecraft:sticky_piston[facing=east]")
    b(2, 0, 7, "minecraft:stone")
    b(0, 0, 7, "minecraft:observer[facing=west]")
    at(4, lambda b, f: b(-1, 0, 7, "minecraft:oak_planks"))
    b(7, 0, 7, "minecraft:piston[facing=east]")
    b(8, 0, 7, "minecraft:stone")
    b(6, 0, 7, "minecraft:observer[facing=west]")
    at(4, lambda b, f: b(5, 0, 7, "minecraft:oak_planks"))


@scenario("piston_redstone", (16, 6, 12))
def _(b, f, at):
    b(1, 0, 1, "minecraft:sticky_piston[facing=east]")
    b(2, 0, 1, "minecraft:redstone_block")
    b(4, 0, 1, "minecraft:redstone_lamp")
    power(b, at, [(0, 0, 1)], on=2, off=10)
    b(1, 0, 4, "minecraft:piston[facing=south]")
    f(2, 0, 4, 6, 0, 4, "minecraft:redstone_wire")
    b(3, 0, 5, "minecraft:piston[facing=south]")
    power(b, at, [(7, 0, 4)], on=3, off=15)
    b(1, 0, 8, "minecraft:piston[facing=east]")
    b(2, 0, 8, "minecraft:observer[facing=east]")
    b(4, 0, 8, "minecraft:redstone_lamp")
    power(b, at, [(0, 0, 8)], on=2, off=20)


@scenario("piston_heads", (12, 6, 10))
def _(b, f, at):
    b(1, 0, 1, "minecraft:piston[facing=east,extended=true]")
    b(2, 0, 1, "minecraft:piston_head[facing=east]")
    b(1, 0, 3, "minecraft:piston[facing=east]")
    b(0, 0, 3, "minecraft:redstone_block")
    at(8, lambda b, f: b(2, 0, 3, "minecraft:air"))
    b(1, 0, 5, "minecraft:piston[facing=east]")
    b(0, 0, 5, "minecraft:redstone_block")
    at(8, lambda b, f: b(1, 0, 5, "minecraft:air"))
    b(1, 0, 7, "minecraft:sticky_piston[facing=east]")
    b(2, 0, 7, "minecraft:stone")
    b(0, 0, 7, "minecraft:redstone_block")
    at(1, lambda b, f: b(0, 0, 7, "minecraft:air"))
    b(7, 0, 3, "minecraft:piston[facing=east]")
    b(6, 0, 3, "minecraft:redstone_block")
    at(2, lambda b, f: b(8, 0, 3, "minecraft:air"))


@scenario("piston_observers", (10, 8, 10))
def _(b, f, at):
    b(2, 3, 3, "minecraft:sticky_piston[facing=east]")
    b(3, 3, 3, "minecraft:slime_block")
    for x, y, z in ((3, 3, 2), (3, 3, 4), (3, 4, 3), (3, 2, 3)):
        b(x, y, z, "minecraft:stone")
    b(3, 3, 1, "minecraft:observer[facing=south]")
    b(3, 3, 5, "minecraft:observer[facing=north]")
    b(3, 5, 3, "minecraft:observer[facing=down]")
    b(3, 1, 3, "minecraft:observer[facing=up]")
    b(4, 3, 1, "minecraft:observer[facing=south]")
    b(4, 5, 3, "minecraft:observer[facing=down]")
    power(b, at, [(1, 3, 3)], on=2, off=12)


def build(only):
    """Lays scenarios out on a grid and returns the timeline description."""
    scenarios = []
    steps = {t: {"tick": t, "commands": [], "snapshot": True} for t in SNAPSHOTS}
    for i, (name, size, fn) in enumerate(SCENARIOS):
        if only and name not in only:
            continue
        ox, oz = 8 + (i % COLUMNS) * GRID, 8 + (i // COLUMNS) * GRID
        oy = SURFACE

        def mk(sink):
            def b(x, y, z, block, mode=""):
                sink.append(f"setblock {ox + x} {oy + y} {oz + z} {block}{' ' + mode if mode else ''}")

            def f(x1, y1, z1, x2, y2, z2, block, mode=""):
                sink.append(f"fill {ox + x1} {oy + y1} {oz + z1} {ox + x2} {oy + y2} {oz + z2} {block}{' ' + mode if mode else ''}")
            return b, f

        setup = []
        b, f = mk(setup)

        def at(tick, cb):
            step = steps.setdefault(tick, {"tick": tick, "commands": [], "snapshot": False})
            cb(*mk(step["commands"]))

        fn(b, f, at)
        steps[0]["commands"].extend(setup)
        sx, sy, sz = size
        scenarios.append({"name": name, "min": [ox - 1, oy - 2, oz - 1], "max": [ox + sx, oy + sy, oz + sz]})
    max_x = max(s["max"][0] for s in scenarios)
    max_z = max(s["max"][2] for s in scenarios)
    return {
        "layers": LAYERS,
        "forceload": [0, 0, max_x + 16, max_z + 16],
        "scenarios": scenarios,
        "steps": [steps[t] for t in sorted(steps)],
    }


class Server:
    def __init__(self, directory, port):
        self.dir = directory
        props = {
            "server-port": port,
            "online-mode": "false",
            "white-list": "false",
            "level-type": "minecraft\\:flat",
            "generator-settings": json.dumps({"layers": [{"block": b, "height": 1} for b in LAYERS], "biome": "minecraft:plains"}).replace(":", "\\:"),
            "generate-structures": "false",
            "spawn-protection": "0",
            "max-tick-time": "-1",
            "pause-when-empty-seconds": "-1",
            "sync-chunk-writes": "false",
            "difficulty": "peaceful",
            "view-distance": "4",
            "simulation-distance": "4",
        }
        (directory / "eula.txt").write_text("eula=true\n", encoding="utf-8")
        (directory / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in props.items()), encoding="utf-8")
        self.p = subprocess.Popen(
            ["java", "-Xmx3G", "-jar", str(WORK / "server.jar"), "--nogui"],
            cwd=directory, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", bufsize=1,
        )
        self.lines = []
        self.lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            with self.lock:
                self.lines.append(line.rstrip())

    def mark(self):
        with self.lock:
            return len(self.lines)

    def since(self, mark):
        with self.lock:
            return self.lines[mark:]

    def wait_for(self, text, timeout, mark=0):
        end = time.time() + timeout
        while time.time() < end:
            for line in self.since(mark):
                if text in line:
                    return line
            if self.p.poll() is not None:
                raise SystemExit("vanilla server exited:\n" + "\n".join(self.since(0)[-30:]))
            time.sleep(0.05)
        raise SystemExit(f"timed out waiting for {text!r}:\n" + "\n".join(self.since(mark)[-20:]))

    def send(self, cmd):
        self.p.stdin.write(cmd + "\n")
        self.p.stdin.flush()

    def game_time(self):
        m = self.mark()
        self.send("time query gametime")
        line = self.wait_for("game time is", 30, m)
        return int(line.split("game time is ", 1)[1].split()[0])

    def stop(self):
        try:
            self.send("stop")
            self.p.wait(timeout=60)
        except Exception:
            self.p.kill()


def run_vanilla(timeline, out, port):
    server_dir = out / "server"
    if server_dir.exists():
        shutil.rmtree(server_dir)
    server_dir.mkdir(parents=True)
    s = Server(server_dir, port)
    try:
        s.wait_for("Done (", 600)
        for rule in ["random_tick_speed 0", "spawn_mobs false", "advance_weather false", "advance_time false",
                     "fire_spread_radius_around_player 0", "max_block_modifications 1000000"]:
            s.send(f"gamerule minecraft:{rule}")
        s.send("weather clear")
        s.send("tick freeze")
        x1, z1, x2, z2 = timeline["forceload"]
        m = s.mark()
        s.send(f"forceload add {x1} {z1} {x2} {z2}")
        s.wait_for("force loaded", 60, m)
        corners = [(x1, z1), (x2, z2), (x1, z2), (x2, z1)]
        end = time.time() + 300
        while True:
            m = s.mark()
            for cx, cz in corners:
                s.send(f"execute if loaded {cx} {SURFACE} {cz} run say loaded {cx} {cz}")
            s.game_time()
            if sum("loaded" in l and "[Server]" in l for l in s.since(m)) == len(corners):
                break
            if time.time() > end:
                raise SystemExit("chunks did not load")
            time.sleep(1)
        g0 = s.game_time()
        region = server_dir / "world" / "dimensions" / "minecraft" / "overworld" / "region"
        snaps = out / "snapshots"
        if snaps.exists():
            shutil.rmtree(snaps)
        failures = []
        tick = 0
        for step in timeline["steps"]:
            t = step["tick"]
            if t > tick:
                s.send(f"tick step {t - tick}")
                end = time.time() + 120
                while s.game_time() < g0 + t:
                    if time.time() > end:
                        raise SystemExit(f"tick step to {t} did not finish")
                    time.sleep(0.05)
                if s.game_time() != g0 + t:
                    raise SystemExit(f"overshot tick {t}")
                tick = t
            m = s.mark()
            for c in step["commands"]:
                s.send(c)
            s.game_time()
            for line in s.since(m):
                if "Could not set" in line or "Unknown" in line or "Incorrect" in line or "not loaded" in line:
                    failures.append((t, line))
            if step["snapshot"]:
                # save-all skips chunks unchanged since the last save, whose saved tick
                # delays would then be stale; touch every chunk (strict: no updates).
                for cx in range(x1 >> 4, (x2 >> 4) + 1):
                    for cz in range(z1 >> 4, (z2 >> 4) + 1):
                        s.send(f"setblock {cx * 16} 318 {cz * 16} minecraft:glass strict")
                        s.send(f"setblock {cx * 16} 318 {cz * 16} minecraft:air strict")
                m = s.mark()
                s.send("save-all flush")
                s.wait_for("Saved the game", 120, m)
                shutil.copytree(region, snaps / f"t{t}")
        return failures
    finally:
        s.stop()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25592)
    ap.add_argument("--out", default=str(WORK / "wp3-blocks" / "diff"))
    ap.add_argument("--only", default="")
    ap.add_argument("--skip-vanilla", action="store_true", help="reuse the snapshots of the last run")
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    only = set(filter(None, a.only.split(",")))
    timeline = build(only)
    (out / "timeline.json").write_text(json.dumps(timeline, indent=1), encoding="utf-8")
    if not a.skip_vanilla:
        failures = run_vanilla(timeline, out, a.port)
        (out / "vanilla_command_failures.json").write_text(json.dumps(failures, indent=1), encoding="utf-8")
        for t, line in failures:
            print(f"vanilla t{t}: {line}")
    r = subprocess.run(["cargo", "run", "--release", "-q", "-p", "kiln-blocks", "--example", "vanilla_diff", "--", str(out)], cwd=ROOT)
    sys.exit(r.returncode)


if __name__ == "__main__":
    main()
