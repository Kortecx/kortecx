
from kortecx.thalamus.utils.catalog_utils import setupDuckDb
from kortecx.utils.wrappers import WrapperClass

class ThalamusRegistry():
    def __init__(self, registryName: str = "kortecxdb"):
        self.registryName = registryName

    def registry_init(self):
        project_configs = WrapperClass().get_config()
        
        if project_configs['UCEnabled'] == False:
            setupDuckDb(defaultDatabase=self.registryName).db_entrypoint()
