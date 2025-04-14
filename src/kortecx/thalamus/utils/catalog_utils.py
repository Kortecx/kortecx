from pydantic import BaseModel
import subprocess
from typing import List, Any, Callable
import numpy as np
import platform
import os
import contextlib
import requests
import duckdb
from duckdb.typing import DuckDBPyType
import inspect

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
    def __init__(self, defaultDatabase, func: Callable):
        self.defaultDatabase = defaultDatabase
        self.func = func

    def db_entrypoint(self):
        duckconn = duckdb.connect(self.defaultDatabase)
        return duckconn
    
    def log_tools(self, tool_name: str, tool_desc: str, group: str = 'kxSchema',traverse: str = "no", params = [Any, Any]):
        connector = self.db_entrypoint()
        caller_file = inspect.getfile(self.func)
        sig = inspect.signature(self.func)
        param_names = list(sig.parameters.keys())
        formatted_params =  "[" + ", ".join(f'"{name}"' for name in param_names) + "]"
        # duckparams: DuckDBPyType = params
        caller_dir = os.path.join(os.path.dirname(os.path.abspath(caller_file)), caller_file)
        create_registry = f"""
                            CREATE SEQUENCE IF NOT EXISTS tools_id_seq; \n
                            CREATE TABLE IF NOT EXISTS {self.defaultDatabase} (idx INTEGER PRIMARY KEY DEFAULT nextval('tools_id_seq'), tool_name VARCHAR, func_name VARCHAR,input_args VARCHAR , tool_location VARCHAR, tool_description VARCHAR,traverse VARCHAR);"""
        existing_table = connector.execute("SHOW TABLES").fetch_df().get("name").values
        if not np.array_equal(existing_table, self.defaultDatabase) or not existing_table:
            connector.execute(create_registry)
        insert_metadata = f"INSERT INTO kortecxdb (tool_name, func_name, input_args, tool_location, tool_description, traverse) values ('{tool_name}','{self.func.__name__}', '{formatted_params}','{caller_dir}','{tool_desc}','{traverse}');"
        existing_tools = connector.execute(f"SELECT * FROM {self.defaultDatabase}").fetch_df().get("tool_name").values
        print(insert_metadata)
        print(existing_tools, tool_name)
        if tool_name not in existing_tools:
            connector.execute(insert_metadata)
        print(f"{self.func.__name__}",self.func, f"{formatted_params}")
        select_stmt = f"SELECT * FROM {self.defaultDatabase}"
        print(connector.execute(select_stmt).fetch_df()) 



if __name__ == '__main__':
    setupDuckDb(defaultDatabase="kortecxdb").db_entrypoint()


