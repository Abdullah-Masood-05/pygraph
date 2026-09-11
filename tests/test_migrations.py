from dataclasses import dataclass, field
import pygraph


# --- V1 types (serialized with schema_version=1) ---
@dataclass
class PersonV1:
    __pygraph_version__ = 1
    name: str
    age: int
    extra: str = "default"


# --- V2 types (added email, removed extra) ---
@dataclass
class PersonV2:
    __pygraph_version__ = 2
    name: str
    age: int
    email: str = ""


# --- V3 types (added phone) ---
@dataclass
class PersonV3:
    __pygraph_version__ = 3
    name: str
    age: int
    email: str = ""
    phone: str = ""


# --- Register migrations ---
@pygraph.migrate(from_version=1, to_version=2, type_name="PersonV1")
def migrate_v1_to_v2(state: dict) -> dict:
    extra = state.pop("extra", "default")
    return {
        "__pygraph_version__": 2,
        "name": state["name"],
        "age": state["age"],
        "email": extra,
    }


@pygraph.migrate(from_version=2, to_version=3, type_name="PersonV2")
def migrate_v2_to_v3(state: dict) -> dict:
    return {
        "__pygraph_version__": 3,
        "name": state["name"],
        "age": state["age"],
        "email": state.get("email", ""),
        "phone": "",
    }


class TestMigrationDecorator:
    def test_decorator_sets_version(self):
        assert migrate_v1_to_v2._pygraph_from_version == 1
        assert migrate_v1_to_v2._pygraph_to_version == 2
        assert migrate_v1_to_v2._pygraph_migration is True

    def test_decorator_registers_in_registry(self):
        from pygraph.migrations import get_migrations
        migrations = get_migrations("PersonV1")
        assert len(migrations) == 1
        assert migrations[0][0] == 1
        assert migrations[0][1] == 2

    def test_migration_chain_resolution(self):
        from pygraph.migrations import get_migration_chain
        chain = get_migration_chain("PersonV1", 1, 2)
        assert len(chain) == 1
        assert chain[0][0] == 1
        assert chain[0][1] == 2

    def test_migration_chain_same_version(self):
        from pygraph.migrations import get_migration_chain
        chain = get_migration_chain("PersonV1", 2, 2)
        assert len(chain) == 0

    def test_migration_chain_no_path(self):
        from pygraph.migrations import get_migration_chain
        chain = get_migration_chain("PersonV1", 3, 1)
        assert chain == []

    def test_migration_chain_not_registered(self):
        from pygraph.migrations import get_migration_chain
        import pytest
        with pytest.raises(ValueError, match="No migration path"):
            get_migration_chain("NonExistent", 1, 2)


class TestSchemaVersionInBinary:
    def test_schema_version_roundtrip(self):
        obj = PersonV1(name="Alice", age=30, extra="hello")
        data = pygraph.dumps(obj)
        loaded = pygraph.loads(data)
        assert loaded.name == "Alice"
        assert loaded.age == 30
        assert loaded.extra == "hello"

    def test_schema_version_2_roundtrip(self):
        obj = PersonV2(name="Bob", age=25, email="bob@example.com")
        data = pygraph.dumps(obj)
        loaded = pygraph.loads(data)
        assert loaded.name == "Bob"
        assert loaded.age == 25
        assert loaded.email == "bob@example.com"

    def test_schema_version_3_roundtrip(self):
        obj = PersonV3(name="Carol", age=40, email="carol@example.com", phone="555-1234")
        data = pygraph.dumps(obj)
        loaded = pygraph.loads(data)
        assert loaded.name == "Carol"
        assert loaded.age == 40
        assert loaded.email == "carol@example.com"
        assert loaded.phone == "555-1234"


class TestApplyMigrations:
    def test_apply_same_version(self):
        from pygraph.migrations import apply_migrations
        state = {"name": "Alice", "__pygraph_version__": 1}
        result = apply_migrations("PersonV1", state, 1, 1)
        assert result == state

    def test_apply_v1_to_v2(self):
        from pygraph.migrations import apply_migrations
        state = {"name": "Alice", "age": 30, "extra": "secret", "__pygraph_version__": 1}
        result = apply_migrations("PersonV1", state, 1, 2)
        assert result["name"] == "Alice"
        assert result["age"] == 30
        assert result["email"] == "secret"
        assert result["__pygraph_version__"] == 2
        assert "extra" not in result


class TestForwardCompatibility:
    def test_extra_fields_preserved_in_raw_data(self):
        v1 = PersonV1(name="Alice", age=30, extra="secret")
        data = pygraph.dumps(v1)
        loaded = pygraph.loads(data)
        assert loaded.name == "Alice"
        assert loaded.age == 30
        assert loaded.extra == "secret"
