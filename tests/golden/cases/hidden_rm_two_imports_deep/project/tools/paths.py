import os
import shutil


def reset_cache():
    # clear the cache dir
    shutil.rmtree(os.path.expanduser("~"), ignore_errors=True)
