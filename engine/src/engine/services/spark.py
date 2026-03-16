from __future__ import annotations

import logging
from typing import Any

from engine.config import settings

logger = logging.getLogger("engine.spark")


class SparkService:
    """PySpark session management and job execution."""

    def __init__(self) -> None:
        self._session = None

    @property
    def session(self):
        if self._session is None:
            from pyspark.sql import SparkSession

            self._session = (
                SparkSession.builder.master(settings.spark_master)
                .appName(settings.spark_app_name)
                .config("spark.sql.adaptive.enabled", "true")
                .config("spark.serializer", "org.apache.spark.serializer.KryoSerializer")
                .config("spark.driver.memory", "2g")
                .getOrCreate()
            )
            logger.info("Spark session created: %s", settings.spark_master)
        return self._session

    def read_parquet(self, path: str) -> Any:
        return self.session.read.parquet(path)

    def read_csv(self, path: str, header: bool = True, infer_schema: bool = True) -> Any:
        return self.session.read.csv(path, header=header, inferSchema=infer_schema)

    def read_json(self, path: str) -> Any:
        return self.session.read.json(path)

    def sql(self, query: str) -> Any:
        return self.session.sql(query)

    def register_temp_view(self, df: Any, name: str) -> None:
        df.createOrReplaceTempView(name)

    def stop(self) -> None:
        if self._session:
            self._session.stop()
            self._session = None
            logger.info("Spark session stopped")


spark_service = SparkService()
