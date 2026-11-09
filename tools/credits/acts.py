#!/usr/bin/env python3
"""Writes the credits' scenes (apps/ta_game/credits/*.toml, in the stage format of
apps/ta_game/src/stage.rs, read by credits_show.rs).

Every scene is a road of cards through a battle: stations (`[[station]]`) a couple of hundred
pixels apart along a route over open ground (a straight one, or an arc round a lake), the armies
of two players drawn up along both of its flanks, facing it, firing across it (the players are
invulnerable, so the fight never ends), aircraft passing over. The game makes each act's camera
from the stations (it looks at each card in its turn), its clearings and its time of day; a scene
says only where and with whom.

The battle is staged off the names. A card is at most the act's `road` wide (credits_show.rs), and
the armies stand clear of it in their lanes either side (`Lanes`): the front lines just outside
it, near enough each other for the rockets, the gauss and the towers to reach across, the guns
behind them, the towers between. Nothing marches down the road: a column there walked over every
name and drew the fire of both lines onto them, scorch marks and all. (The game's test
`the_credits_scenes_put_every_unit_where_it_can_stand` holds every unit, where it ends up
standing, off every card's ground.)

    python tools/credits/acts.py [id ...]       (from the repository root)

The routes were found by a search of each map's heights and features for open, flat, dry,
tree-free ground (a line, or an arc round a lake), then looked at in the game.
"""
import math
import os
import sys

OUT = os.path.join(os.path.dirname(__file__), '..', '..', 'apps', 'ta_game', 'credits')


def basis(heading):
    """The way the camera looks along the route (forward) and its right, as (x, z) unit vectors."""
    h = math.radians(heading)
    return (math.sin(h), -math.cos(h)), (math.cos(h), math.sin(h))


def add(p, v, k):
    return (p[0] + v[0] * k, p[1] + v[1] * k)


# The widest a card is laid (px; credits_show.rs's ROAD unless an act says).
ROAD = 400


class Lanes:
    """Where the armies stand either side of a road `road` wide (px from its middle), clear of the
    widest card by a few px more than a unit's half: the front lines (tanks and kbots up to 3x3
    cells; beside them those that reach across, 2x2, a little nearer: under 400 px apart, the reach
    of the rockets (Rocko, Storm), the gauss (Fido) and the heavy laser towers), the heavy guns
    (the Punisher, 4 cells long), the towers a little further out and between the front lines'
    groups, the artillery behind."""

    def __init__(self, road):
        half = road / 2
        self.front = half + 30
        self.reach = half + 20
        self.heavy = half + 40
        self.towers = half + 34
        self.guns = half + 100


class Scene:
    def __init__(self, id, map, title, hours, themes, style='burn', seed=1, height=250, back=330,
                 fov=40, clear=300, moon=None, rain=None, floor=None, searchlights=0, road=340):
        self.id, self.map, self.title = id, map, title
        self.hours, self.themes, self.style, self.seed = hours, themes, style, seed
        self.height, self.back, self.fov, self.clear = height, back, fov, clear
        self.road = road
        self.lanes = Lanes(road)
        self.moon, self.rain = moon, rain
        self.floor, self.searchlights = floor, searchlights
        self.stations = []          # (x, z, heading)
        self.styles = None          # card styles in turn, if not all the scene's
        self.blasts = {}            # station -> (x, z) of the blast that sets its card alight
        self.units = []
        self.orders = []
        self.events = []
        self.features = []

    # -- the route
    def straight(self, start, heading, n, spacing):
        f, _ = basis(heading)
        self.stations = [(*add(start, f, i * spacing), heading) for i in range(n)]
        return self

    def arc(self, centre, radius, start_deg, sign, n, spacing):
        """Stations round `centre`, `sign` +1 clockwise on the map (x east, z south) from the angle
        `start_deg` (measured as atan2(z, x)); heading along the arc."""
        step = spacing / radius
        self.stations = []
        for i in range(n):
            a = math.radians(start_deg) + sign * step * i
            x, z = centre[0] + math.cos(a) * radius, centre[1] + math.sin(a) * radius
            tx, tz = -math.sin(a) * sign, math.cos(a) * sign
            self.stations.append((x, z, round(math.degrees(math.atan2(tx, -tz)) % 360)))
        return self

    @property
    def n(self):
        return len(self.stations)

    def here(self, i):
        x, z, h = self.stations[i]
        return (x, z), h

    def along(self, i, lateral=0.0, ahead=0.0):
        """A place by the route: station i, `lateral` to its right, `ahead` of it."""
        p, h = self.here(i)
        f, r = basis(h)
        return add(add(p, r, lateral), f, ahead)

    def heading(self, i):
        return self.stations[i][2]

    # -- the cast
    def line(self, side, kind, at, facing, count=1, spacing=40, rows=1, label=None, **extra):
        u = dict(type=kind, side=side, at=at, heading=round(facing % 360), count=count, spacing=spacing, rows=rows)
        if label:
            u['label'] = label
        u.update(extra)
        self.units.append(u)

    def flank(self, side, kinds, lateral, every=1, offset=0.0, count=4, spacing=40, rows=1, first=0, last=None, skip=(), out=None, **extra):
        """A line of units for every station (or every `every`th), cycling through `kinds`, on the
        side of the route (lateral < 0 left, > 0 right), facing it; none at the stations `skip`
        names, further out at those `out` names ({station: lateral}: up a terrace, past a cliff)."""
        for i in range(first, self.n if last is None else last, every):
            if i in skip:
                continue
            at = (out or {}).get(i, lateral)
            facing = self.heading(i) + (90 if at < 0 else -90)
            kind = kinds[(i // every) % len(kinds)]
            self.line(side, kind, self.along(i, at, offset), facing, count, spacing, rows, **extra)

    def order(self, **kw):
        self.orders.append(kw)

    def event(self, **kw):
        self.events.append(kw)

    # -- the text
    def text(self):
        out = []
        w = out.append
        w(f'# The credits\' scene {self.id} (tools/credits/acts.py writes it; credits_show.rs plays it).')
        w(f'# {self.title}: a road of cards through a battle on {self.map}.')
        w(f'map = "{self.map}"')
        w(f'seed = {self.seed}')
        w('missing = "skip"')
        if self.moon:
            w(f'moon = [{self.moon[0]}, {self.moon[1]}]')
        if self.rain is not None:
            w(f'rain = {"true" if self.rain else "false"}')
        w('')
        w('[act]')
        w(f'title = "{self.title}"')
        w('themes = [' + ', '.join(f'"{t}"' for t in self.themes) + ']')
        if self.hours:
            w(f'hours = [{self.hours[0]}, {self.hours[1]}]')
        w(f'style = "{self.style}"')
        w(f'height = {self.height}')
        w(f'back = {self.back}')
        w(f'fov = {self.fov}')
        w(f'clear = {self.clear}')
        if self.road != ROAD:
            w(f'road = {self.road}')
        if self.floor is not None:
            w(f'floor = {self.floor}')
        if self.searchlights:
            w(f'searchlights = {self.searchlights}')
        w('')
        for p in (0, 1):
            w('[[player]]')
            w(f'index = {p}')
            w('invulnerable = true')
            w('')
        for i, (x, z, h) in enumerate(self.stations):
            w('[[station]]')
            w(f'at = [{x:.0f}, {z:.0f}]')
            w(f'heading = {h}')
            if self.styles and self.styles[i % len(self.styles)] != self.style:
                w(f'style = "{self.styles[i % len(self.styles)]}"')
            if i in self.blasts:
                bx, bz = self.blasts[i]
                w(f'blast = [{bx:.0f}, {bz:.0f}]')
            w('')
        for f in self.features:
            w('[[feature]]')
            for k, v in f.items():
                w(f'{k} = "{v}"' if isinstance(v, str) else f'{k} = [{v[0]:.0f}, {v[1]:.0f}]' if isinstance(v, (list, tuple)) else f'{k} = {v}')
            w('')
        for u in self.units:
            w('[[unit]]')
            w(f'type = "{u["type"]}"')
            w(f'side = {u["side"]}')
            w(f'at = [{u["at"][0]:.0f}, {u["at"][1]:.0f}]')
            w(f'heading = {u["heading"]}')
            if u.get('count', 1) > 1:
                w(f'count = {u["count"]}')
                sp = u['spacing']
                w(f'spacing = {sp}' if not isinstance(sp, list) else f'spacing = [{sp[0]}, {sp[1]}]')
            if u.get('rows', 1) > 1:
                w(f'rows = {u["rows"]}')
            if u.get('label'):
                w(f'label = "{u["label"]}"')
            for k in ('airborne', 'altitude', 'built', 'builder', 'build_seconds', 'rise_at', 'hold', 'stockpile'):
                if k in u:
                    v = u[k]
                    w(f'{k} = {"true" if v is True else "false" if v is False else (chr(34) + v + chr(34)) if isinstance(v, str) else v}')
            w('')
        for o in self.orders:
            w('[[order]]')
            for k, v in o.items():
                if isinstance(v, str):
                    w(f'{k} = "{v}"')
                elif isinstance(v, bool):
                    w(f'{k} = {"true" if v else "false"}')
                elif isinstance(v, (list, tuple)):
                    w(f'{k} = [' + ', '.join(f'{c:.0f}' if isinstance(c, float) else str(c) for c in v) + ']')
                else:
                    w(f'{k} = {v}')
            w('')
        for e in self.events:
            w('[[event]]')
            for k, v in e.items():
                if isinstance(v, str):
                    w(f'{k} = "{v}"')
                elif isinstance(v, (list, tuple)):
                    w(f'{k} = [' + ', '.join(f'{c:.0f}' if isinstance(c, float) else str(c) for c in v) + ']')
                else:
                    w(f'{k} = {v}')
            w('')
        return '\n'.join(out)


# ---- The scenes ----------------------------------------------------------------------------

ARM_LINE = ['ARMBULL', 'ARMZEUS', 'ARMHAM', 'ARMPW']
CORE_LINE = ['CORRAID', 'CORSTORM', 'CORTHUD', 'CORAK']
# What reaches across the road from one front line to the other (400 px and more).
ARM_REACH = ['ARMROCK', 'ARMFIDO']
CORE_REACH = ['CORSTORM']


def front(s, side, cast, reach, count=2, spacing=46, **extra):
    """`side`'s front line at every station: `count` of its cast (one kind a station, in turn) and
    as many beside them that reach across the road, the cast nearer the camera."""
    sign = -1 if side == 0 else 1
    s.flank(side, cast, sign * s.lanes.front, offset=-spacing, count=count, spacing=spacing, **extra)
    s.flank(side, reach, sign * s.lanes.reach, offset=spacing, count=count, spacing=spacing, **extra)


def highland_dawn():
    s = Scene('highland_dawn', 'Highland Rift', 'First light over the highlands', (5.7, 7.2),
              ['original', 'project', 'lead', 'producer', 'art', 'design'], seed=21, rain=False)
    s.straight((14675, 9728), 0, 10, 230)
    s.styles = ['burn', 'burn', 'light', 'burn', 'burn', 'light', 'burn', 'burn', 'light', 'burn']
    L = s.lanes
    # The Arm on the left of the road, the Core on the right, each facing it: tanks and kbots in
    # front with rockets and gauss beside them, rockets and artillery behind, a tower between.
    front(s, 0, ARM_LINE, ARM_REACH)
    s.flank(0, ['ARMMERL', 'ARMROCK'], -L.guns, offset=90, count=2, spacing=62)
    s.flank(0, ['ARMLLT'], -L.towers, every=3, offset=115, count=1)
    front(s, 1, CORE_LINE, CORE_REACH)
    s.flank(1, ['CORMART', 'CORVROC'], L.guns, offset=90, count=2, spacing=62)
    s.flank(1, ['CORHLT'], L.towers, every=3, offset=115, count=1)
    return s


def foundry_works():
    """The works at their morning shift: both sides' constructors raise their bases beside the road
    in the stream of their nanolathes, a card coming up out of the deck as each is built."""
    s = Scene('foundry_works', 'Foundry Deck', 'The shift begins at the foundry', (9.0, 10.2),
              ['programming', 'engine', 'network', 'editor', 'production'], style='nano', seed=8)
    s.straight((2816, 1184), 247, 6, 200)
    L = s.lanes
    # (The lines between the works: one of the cast, one that reaches across, between two
    # stations; none where a work is moved back off a raised block, below.)
    for side, cast, reach, skip in ((0, ARM_LINE, ARM_REACH, (2,)), (1, CORE_LINE, CORE_REACH, (4,))):
        sign = -1 if side == 0 else 1
        s.flank(side, cast, sign * L.front, offset=100, count=1, last=s.n - 1, skip=skip)
        s.flank(side, reach, sign * L.reach, offset=144, count=1, last=s.n - 1, skip=skip)
    arm = ['ARMSOLAR', 'ARMWIN', 'ARMLLT', 'ARMSOLAR', 'ARMRAD', 'ARMWIN', 'ARMLLT']
    core = ['CORSOLAR', 'CORWIN', 'CORLLT', 'CORSOLAR', 'CORRAD', 'CORWIN', 'CORLLT']
    # (The works stand clear of the cards beside each station, their constructors behind them;
    # off the raised blocks of the deck, which lie on the Arm's side by the fourth station and on
    # the Core's by the sixth.)
    # (A solar collector is 5 cells across, and a building stands on whole cells.)
    work = s.road / 2 + 40 + 16
    ahead = {(0, 3): -40, (1, 5): -60}
    for i in range(s.n):
        t = 0.8 + 4.6 * i
        for side, builder, kinds in ((0, 'ARMCK', arm), (1, 'CORCK', core)):
            sign = -1 if side == 0 else 1
            label = f'b{side}{i}'
            off = ahead.get((side, i), 20)
            s.line(side, builder, s.along(i, sign * (work + 80), off + 20), s.heading(i) + (90 if sign < 0 else -90), label=label)
            s.order(t=round(t, 1), units=label, do='build', type=kinds[i], at=s.along(i, sign * work, off), seconds=4.5, label=f'frame{side}{i}')
    return s


def cinder_night():
    """The Core's guns hold the south shore of the lava lake against the Arm's: Doomsday, Punishers,
    Berthas, the lake's glow on the road (only units that stand on the basalt: see the probe in
    the credits' notes). The road runs between the lake and a basalt terrace that comes in over the
    Core's flank by the eighth station: the Core's line goes up on it there."""
    s = Scene('cinder_night', 'Cinder Basin', 'Night falls on the basalt', (21.9, 22.7),
              ['music', 'sound', 'composition', 'art', 'artist', 'thanks'], seed=12, moon=(20, 26),
              floor=0.04, searchlights=3)
    s.straight((8192, 8534), 90, 10, 230)
    s.styles = ['burn', 'paint', 'burn', 'paint', 'paint', 'burn']
    L = s.lanes
    front(s, 0, ['ARMZEUS', 'ARMHAM', 'ARMPW', 'ARMZEUS'], ARM_REACH)
    s.flank(0, ['ARMBRTHA'], -L.guns, every=2, offset=115, count=1)
    s.flank(0, ['ARMLLT'], -L.towers, every=3, offset=115, count=1)
    s.flank(1, ['CORPUN'], L.heavy, offset=-50, count=2, spacing=72, out={7: L.heavy + 60})
    s.flank(1, CORE_REACH, L.reach, offset=50, count=2, spacing=46, out={7: L.heavy + 60, 8: L.reach + 25})
    s.flank(1, ['CORTOAST', 'CORHLT'], L.guns, every=2, offset=115, count=1, out={6: L.guns + 70}, skip=(8,))
    s.flank(1, ['CORHLT'], L.towers, every=3, offset=115, count=1, skip=(6,))
    # The Doomsday machine on the terrace over the road.
    s.line(1, 'CORDOOM', s.along(7, L.guns + 60, 0), s.heading(7) - 90, 1)
    return s


def sunscar_noon():
    """Noon over the dunes: the Arm's air raids the Core's guns as the tanks trade fire across the
    sand. The road runs through the gap between two plateaus, turned to keep off both (the
    plateau over the Core's flank by the fourth station, the Arm's by the eighth)."""
    s = Scene('sunscar_noon', 'Sunscar Dunes', 'High noon over the dunes', (11.6, 12.6),
              ['testing', 'quality', 'test', 'playtest', 'localization', 'languages'], seed=33)
    s.straight((4234, 2873), 260, 10, 230)
    L = s.lanes
    front(s, 0, ['ARMBULL', 'ARMFIDO', 'ARMZEUS', 'ARMSTUMP'], ARM_REACH)
    s.flank(0, ['ARMMERL', 'ARMROCK'], -L.guns, offset=90, count=2, spacing=62, skip=(7,))
    front(s, 1, ['CORRAID', 'CORAK', 'CORTHUD', 'CORSTORM'], CORE_REACH)
    s.flank(1, ['CORVROC'], L.guns, offset=90, count=2, spacing=62, first=1, last=8, skip=(3,))
    s.flank(1, ['CORHLT'], L.towers, every=4, offset=115, count=1)
    # Bombers in along the Core's guns, in three passes: down their line, not across the road
    # (a stick of bombs falls along the run, before the target as after it).
    for k, t in enumerate((6.0, 20.0, 34.0)):
        i1 = min(s.n - 2, 5 + 3 * k)
        s.line(0, 'ARMTHUND', s.along(0, L.guns + 40, -500), s.heading(0), 3, 70, airborne=True, altitude=130, label=f'air{k}')
        s.order(t=t, units=f'air{k}', do='attack', at=s.along(i1, L.guns, 90))
    return s


def frostfall_night():
    """A snowbound hollow by moonlight: the lines fight across the road, the snow takes the light of
    the fires."""
    s = Scene('frostfall_night', 'Frostfall Hollow', 'Moonlight on the snow', (23.0, 23.8),
              ['thanks', 'special', 'community', 'tools', 'built'], seed=44, moon=(60, 38), floor=0.3)
    s.straight((7424, 4544), 180, 10, 230)
    s.styles = ['burn']
    L = s.lanes
    front(s, 0, ['ARMBULL', 'ARMZEUS', 'ARMHAM', 'ARMFIDO'], ARM_REACH)
    s.flank(0, ['ARMMERL', 'ARMROCK'], -L.guns, offset=100, count=2, spacing=62)
    s.flank(0, ['ARMLLT'], -L.towers, every=3, offset=115, count=1)
    front(s, 1, ['CORRAID', 'CORAK', 'CORSTORM', 'CORTHUD'], CORE_REACH)
    s.flank(1, ['CORMART', 'CORVROC'], L.guns, offset=100, count=2, spacing=62)
    s.flank(1, ['CORHLT'], L.towers, every=3, offset=115, count=1)
    return s


def starfall_finale():
    """The road starts past the crater's rim (a card on the rim was laid over its step)."""
    s = Scene('starfall_finale', 'Rift Belt', 'The last light, among the asteroids', (16.6, 18.4), ['finale'], seed=5)
    s.straight((943, 1972), 68, 7, 200)
    s.styles = ['burn']
    L = s.lanes
    last = s.n - 1
    # (No lines at the emblem's station: the two Commanders face each other across it.)
    front(s, 0, ARM_LINE, ARM_REACH, last=last)
    s.flank(0, ['ARMMERL', 'ARMROCK'], -L.guns, offset=100, count=2, spacing=62, last=last)
    front(s, 1, CORE_LINE, CORE_REACH, last=last)
    s.flank(1, ['CORMART', 'CORVROC'], L.guns, offset=100, count=2, spacing=62, last=last)
    s.line(0, 'ARMCOM', s.along(last, -L.front, 0), s.heading(last) + 90)
    s.line(1, 'CORCOM', s.along(last, L.front, 0), s.heading(last) - 90)
    return s


ACTS = {'frostfall_night': frostfall_night, 'highland_dawn': highland_dawn, 'foundry_works': foundry_works, 'cinder_night': cinder_night, 'sunscar_noon': sunscar_noon, 'starfall_finale': starfall_finale}


def main():
    wanted = sys.argv[1:] or list(ACTS)
    os.makedirs(OUT, exist_ok=True)
    for id in wanted:
        text = ACTS[id]().text()
        path = os.path.normpath(os.path.join(OUT, id + '.toml'))
        with open(path, 'w', encoding='utf-8', newline='\n') as f:
            f.write(text + '\n')
        print(path, len(text.splitlines()), 'lines')


if __name__ == '__main__':
    main()
