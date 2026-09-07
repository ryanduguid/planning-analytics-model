"""An open IBM Planning Analytics budgeting model and an offline engine for it."""

from importlib.metadata import PackageNotFoundError, version as _installed_version

try:
    __version__ = _installed_version("pacioliscube")
except PackageNotFoundError:  # running from a source tree without installation
    __version__ = "0.0.0.dev0"

__all__ = ["__version__"]
