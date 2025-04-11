import json
import os
class WrapperClass():
    def __init__(self):
        pass

    def get_config(self):
        with open(os.getcwd() + "/kortecx.config.json","r") as registry_conf:
            _configs = json.load(registry_conf)
        return _configs