import os
from os import path
import re

path_regex = re.compile(r'.*path="([a-z0-9_\/\.-]*)".*')


class Project:
    def __init__(self, fp: str) -> None:
        self.path = fp
        self.dir = path.normpath(path.dirname(fp))
        self.deps: list[Project] = []
        self.needed_by: list[Project] = []

    def __str__(self) -> str:
        return "Proj(%s) -> %s" % (self.dir, self.deps)

    def __repr__(self) -> str:
        return str(self)

    def needs(self, other: str) -> bool:
        return any(map(lambda v: other in v.dir, self.deps))

    def is_needed_by(self, other: str) -> bool:
        return any(map(lambda v: other in v.dir, self.needed_by))


def read_file(p: str) -> str:
    with open(p, "r") as f:
        return f.read()


# gather projects (all directories with `Cargo.toml`)
p = filter(lambda v: "Cargo.toml" in v[2], os.walk("."))
p = map(lambda v: Project(path.join(v[0], "Cargo.toml")), p)
projects = list(p)
n = range(len(projects))


def parse_cargo_toml(fp: str) -> list[str]:
    lines = read_file(fp).splitlines()
    lines = map(lambda v: v.strip(), lines)
    lines = filter(lambda v: not v.startswith("#"), lines)
    lines = map(lambda v: v.replace(" ", ""), lines)

    def parse_path(line: str) -> str:
        hit = path_regex.match(line)
        if "path=" in line and hit is not None:
            dep = hit.groups()[0]
            dep = path.normpath(path.join(path.dirname(fp), dep))
            if path.isdir(dep):
                return dep

    lines = map(parse_path, lines)
    lines = filter(lambda v: v is not None, lines)
    lines = list(lines)

    return lines


# update projects such that i needs j.
def update(projects: list[Project], i: int, dep: str):
    for j in n:
        if projects[j].dir == dep:
            projects[i].deps.append(projects[j])
            projects[j].needed_by.append(projects[i])
            return
    raise ("cannot find %s" % dep)


for i in n:
    for dep in parse_cargo_toml(projects[i].path):
        update(projects, i, dep)


def show_unused(projects: list[Project]):
    unneeded = list(filter(lambda v: len(v.needed_by) == 0, projects))
    print("[unneeded]")
    for p in unneeded:
        print("*", p.dir)


def show_1_dep(projects: list[Project]):
    print("[needed by only 1]")
    for p in projects:
        if len(p.needed_by) == 1:
            print("[1]", p.dir, dirs(p.needed_by))


def dirs(ps: list[Project]):
    return list(map(lambda v: v.dir, ps))


show_unused(projects)
show_1_dep(projects)

for i, p in enumerate(projects):
    if p.dir == "wezterm":
        if i > 0:
            projects[i], projects[0] = projects[0], projects[i]

ht = {p.dir: p for p in projects}


def walk(p_dir: str, depth=0):
    p = ht[p_dir]
    print(" " * depth * 2, "*", p.dir)
    for p in p.deps:
        walk(p.dir, depth + 1)


walk("term")
