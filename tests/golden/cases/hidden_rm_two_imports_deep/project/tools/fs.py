import shutil
from tools import paths


def tidy(name):
    shutil.rmtree(name, ignore_errors=True)
    paths.reset_cache()
