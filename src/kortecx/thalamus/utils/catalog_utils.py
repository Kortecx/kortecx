from pydantic import BaseModel
import subprocess
from typing import List, Optional
import platform
import os
import contextlib
import requests
import duckdb

class SetupUCDB():
    def __init__(self, defaultModel: str = "Kortecx"):
        self.defaultModel = defaultModel

        self.mac_path = os.path.expanduser("~/.kortecx")
        if self.mac_path:
            catalog_path = self.mac_path + "/unitycatalog"
            catalog_executor = catalog_path + "/bin"

    def UCClone(self):
        platform_type = platform.system()
        if platform_type == 'Darwin':
            listUCExists = os.listdir(os.path.expanduser("~/."))
            print(listUCExists)
            if ".kortecx" not in listUCExists:
                os.mkdir(self.mac_path)
                os.chdir(self.mac_path)
                print("changed dir")
                if "unitycatalog" not in os.path.expanduser("~/.") + "/.kortecx":
                    subprocess.run(["git","clone","--quiet","https://github.com/unitycatalog/unitycatalog.git"])
                    os.chdir(os.path.expanduser("~/.") + "/.kortecx" + "/unitycatalog")
                    subprocess.run(["sbt","clean","compile"])

    def startUC(self):
        SetupUCDB().UCClone()
        try:
            subprocess.run([f"{self.catalog_executor}/start-uc-server","-p","8213"], capture_output=True, text=True)
        except Exception as e:
            print(f"Failed to setup thalamus registry with exception {e} \n " \
                    "Make sure to Java17 and SBT are installed")
            
    def checkUCStatus(self):
        try:
            resp = requests.get("http://localhost:8213")
            if resp.text() == "Hello, Unity Catalog!":
                print("thalamus running")
        except:
            SetupUCDB().startUC()


class UCInteractor(BaseModel):
    @contextlib.contextmanager
    def list_catalogs(self, catalog_name: str):
        mac_path = os.path.expanduser("~/.kortecx") + "/unitycatalog"
        os.chdir(mac_path)
        catalogs = subprocess.run([f"{mac_path}/bin/uc","catalog","list"])
        print(catalogs)

class setupDuckDb():
    def __init__(self, defaultDatabase):
        self.defaultDatabase = defaultDatabase

    def db_entrypoint(self):
        duckconn = duckdb.connect(self.defaultDatabase)
        return duckconn



if __name__ == '__main__':
    setupDuckDb(defaultDatabase="Kortecx.db").db_entrypoint()


