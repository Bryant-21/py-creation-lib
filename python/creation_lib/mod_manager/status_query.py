from .backend import call


OPTIONS = {
    "conflicts": (("all", "All file states"), ("conflicting", "Any file conflict"), ("wins", "Wins files"),
                  ("loses", "Loses files"), ("mixed", "Wins and loses"), ("uncontested", "Uncontested"),
                  ("inactive", "Disabled / hidden providers"), ("empty", "No indexed files"),
                  ("unavailable", "Source unavailable"), ("unknown", "Unknown / partial inventory")),
    "updates": (("all", "All update states"), ("published", "Published file replacement"),
                ("different", "Mod version differs"), ("matching", "Mod version matches"), ("unknown", "Update unknown")),
    "endorsement": (("all", "All endorsement states"), ("endorsed", "Endorsed"),
                    ("unendorsed", "Unendorsed"), ("unknown", "Endorsement unknown")),
    "tracking": (("all", "All tracking states"), ("tracked", "Tracked"),
                 ("untracked", "Untracked"), ("unknown", "Tracking unknown")),
}


def normalized(filters=None):
    return call("status_normalized", filters)


def inventory(rows, index, updates, accounts):
    return call("status_inventory", rows, index, updates, accounts)


def matches(row, states, filters):
    return call("status_matches", row.id, states, filters)


def sort_key(row, states, field):
    return call("status_sort_key", row.id, states, field)
