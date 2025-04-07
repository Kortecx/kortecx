import click
import os
from kortecx.cli.utils.config_handler import ConfigHandler
from kortecx.thalamus.registry import ThalamusRegistry

@click.group()
def kx():
    "Kortecx CLI"
    pass

@kx.command()
@click.argument("project_name")
def init(project_name):
    print("testing")
    ConfigHandler().generate_configs(project_name=project_name)
    ThalamusRegistry().registry_init()


if __name__ == '__main__':
    kx()