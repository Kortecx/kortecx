
import json
import os
from kortecx.thalamus.utils.catalog_utils import setupDuckDb, SetupUCDB

class ThalamusRegistry():
    def __init__(self, registryName: str = "Kortecx.db"):
        self.registryName = registryName

    def registry_init(self):
        with open(os.getcwd() + "/kortecxconfig.json","r") as registry_conf:
            _configs = json.load(registry_conf)
        
        if _configs['UCEnabled'] == False:
            duckconn = setupDuckDb(defaultDatabase=self.registryName).db_entrypoint()
