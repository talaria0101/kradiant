#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include "../kradiant.h"

void test_simple_box() {
    printf("Testing simple_box.map...\n");
    KR_EditorState* ed = kr_editor_new();
    assert(ed != NULL);

    bool ok = kr_editor_load_map(ed, "../test/simple_box.map");
    assert(ok);

    int ent_count = kr_editor_get_entity_count(ed);
    assert(ent_count == 1);

    char* classname = kr_editor_get_entity_property(ed, 0, "classname");
    assert(strcmp(classname, "worldspawn") == 0);
    kr_free_string(classname);

    KR_Map* map = kr_editor_get_map_ptr(ed);
    KR_Entity* world = kr_map_get_entity(map, 0);
    assert(kr_entity_get_brush_count(world) == 1);

    KR_Brush* brush = kr_entity_get_brush(world, 0);
    assert(kr_brush_get_face_count(brush) == 6);

    char* tex = kr_brush_get_face_texture(brush, 0);
    assert(strcmp(tex, "common/caulk") == 0);
    kr_free_string(tex);

    KR_Vec3 p1 = kr_brush_get_face_plane_point(brush, 0, 0);
    // Simple box: ( 288 264 -146 ) ( 288 152 -146 ) ( 312 152 -146 )
    assert(p1.x == 288.0f);
    assert(p1.y == 264.0f);
    assert(p1.z == -146.0f);

    kr_editor_free(ed);
    printf("simple_box.map test passed!\n");
}

void test_multi_entity() {
    printf("Testing multi_entity.map...\n");
    KR_EditorState* ed = kr_editor_new();
    assert(ed != NULL);

    bool ok = kr_editor_load_map(ed, "../test/multi_entity.map");
    assert(ok);

    int ent_count = kr_editor_get_entity_count(ed);
    assert(ent_count == 5);

    char* c0 = kr_editor_get_entity_property(ed, 0, "classname");
    assert(strcmp(c0, "worldspawn") == 0);
    kr_free_string(c0);

    char* c1 = kr_editor_get_entity_property(ed, 1, "classname");
    assert(strcmp(c1, "mp_deathmatch_intermission") == 0);
    kr_free_string(c1);

    char* origin = kr_editor_get_entity_property(ed, 1, "origin");
    assert(strcmp(origin, "-124 -16 16") == 0);
    kr_free_string(origin);

    char* c4 = kr_editor_get_entity_property(ed, 4, "classname");
    assert(strcmp(c4, "trigger_use") == 0);
    kr_free_string(c4);

    kr_editor_free(ed);
    printf("multi_entity.map test passed!\n");
}

int main() {
    printf("Starting Kradiant C FFI Tests...\n");
    printf("Library Version: %s\n", kr_get_version());

    test_simple_box();
    test_multi_entity();

    printf("All tests passed successfully!\n");
    return 0;
}
