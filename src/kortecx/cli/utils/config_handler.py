import os
import json
from pydantic import BaseModel

class ConfigHandler():

    def __init__(self):
        pass

    def generate_configs(self, project_name: str):
        config_path = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),"utils/defaultconfigs.json")
        with open(config_path,'r') as configs:
            default_configs = json.load(configs)
            default_configs['name'] = project_name
        validate_configs = os.listdir(os.getcwd())
        if "kortecxconfig.json" not in validate_configs:
            with open(os.getcwd() + "/kortecxconfig.json", "w") as cfg:
                json.dump(default_configs, cfg, indent=4)


if __name__ == '__main__':
    ConfigHandler().generate_configs("sample")