"""An open IBM Planning Analytics budgeting model and an offline engine for it."""

from importlib.metadata import version as _installed_version

__version__ = _installed_version("pacioliscube")

__all__ = ["__version__"]
