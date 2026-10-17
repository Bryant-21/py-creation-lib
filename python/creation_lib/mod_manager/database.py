from .backend import call


class Row:
    def __init__(self, names, values):
        self._names, self._values = names, values

    def keys(self):
        return list(self._names)

    def __getitem__(self, key):
        return self._values[self._names.index(key) if isinstance(key, str) else key]

    def __iter__(self):
        return iter(self._values)

    def __len__(self):
        return len(self._values)


class Cursor:
    def __init__(self, result):
        names, values, self.rowcount, self.lastrowid = result
        self._rows = iter(Row(names, value) for value in values)

    def __iter__(self):
        return self._rows

    def fetchone(self):
        return next(self._rows, None)

    def fetchall(self):
        return list(self._rows)


class Database:
    def __init__(self, path, readonly=False):
        self.native = call("Database", path, readonly)

    def execute(self, sql, parameters=()):
        return Cursor(self.native.execute(sql, parameters))

    def executemany(self, sql, parameters):
        result = None
        for values in parameters:
            result = self.execute(sql, values)
        return result

    def executescript(self, sql):
        return self.native.executescript(sql)

    def commit(self):
        return self.native.commit()

    def rollback(self):
        return self.native.rollback()

    def close(self):
        self.native.close()

    def __enter__(self):
        return self

    def __exit__(self, kind, value, traceback):
        if kind is None:
            self.commit()
        else:
            self.rollback()
