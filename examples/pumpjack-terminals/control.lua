-- Measures where a pumpjack's fluid output actually is, per rotation, by
-- placing four of them in a real game and asking the runtime API.
--
-- The dump tells us the prototype's pipe_connections.positions changed between
-- 2.0.77 and 2.1.14. It does not tell us where the connecting pipe goes, which
-- is what the planner hardcodes. target_position answers that directly, so
-- nothing has to be derived from the tile-offset convention.

local function collect()
    local surface = game.surfaces[1]
    local force = game.forces.player

    local dirs = {
        { name = "north", value = defines.direction.north },
        { name = "east", value = defines.direction.east },
        { name = "south", value = defines.direction.south },
        { name = "west", value = defines.direction.west },
    }

    local out = {
        direction_values = {
            north = defines.direction.north,
            east = defines.direction.east,
            south = defines.direction.south,
            west = defines.direction.west,
        },
        active_mods = script.active_mods,
        pumpjacks = {},
        errors = {},
    }

    -- The default generated area is small, and these sit outside it.
    for i = 1, #dirs do
        surface.request_to_generate_chunks({ i * 16, 0 }, 2)
    end
    surface.force_generate_chunk_requests()

    for i, d in ipairs(dirs) do
        local cx, cy = i * 16, 0

        -- A pumpjack needs oil under it, so lay a 3x3 patch first.
        for dx = -1, 1 do
            for dy = -1, 1 do
                pcall(function()
                    surface.create_entity {
                        name = "crude-oil",
                        position = { cx + dx, cy + dy },
                        amount = 100000,
                    }
                end)
            end
        end

        local ok, entity = pcall(function()
            return surface.create_entity {
                name = "pumpjack",
                position = { cx, cy },
                direction = d.value,
                force = force,
            }
        end)

        if not ok or not entity then
            table.insert(out.errors, d.name .. ": " .. tostring(entity))
        else
            local ex, ey = entity.position.x, entity.position.y
            local entry = {
                direction_name = d.name,
                direction_value = d.value,
                requested_direction = d.value,
                actual_direction = entity.direction,
                position = { x = ex, y = ey },
                mirroring = entity.mirroring,
                connections = {},
                prototype_positions = {},
            }

            -- 2.1 deleted LuaEntity.fluidbox and the whole LuaFluidBox class,
            -- flattening it onto LuaEntity as get_fluid_box_*. Ask for the
            -- accessor rather than branching on a version string.
            -- Reading an unknown key on LuaEntity raises in 2.0 rather than
            -- returning nil, so the feature check itself has to be guarded.
            local boxes = entity.prototype.fluidbox_prototypes
            local get_connections
            local has_new = pcall(function()
                return entity.get_fluid_box_pipe_connections
            end)
            if has_new then
                entry.api = "2.1 get_fluid_box_pipe_connections"
                get_connections = function(i)
                    return entity.get_fluid_box_pipe_connections(i)
                end
            else
                entry.api = "2.0 fluidbox.get_pipe_connections"
                local box = entity.fluidbox
                get_connections = function(i)
                    return box.get_pipe_connections(i)
                end
            end

            for idx = 1, #boxes do
                for _, c in ipairs(get_connections(idx)) do
                    table.insert(entry.connections, {
                        box_index = idx,
                        flow_direction = c.flow_direction,
                        connection_type = c.connection_type,
                        -- Offsets are what the planner cares about; absolute
                        -- coordinates depend on where we happened to build.
                        connection_offset = {
                            x = c.position.x - ex,
                            y = c.position.y - ey,
                        },
                        target_offset = {
                            x = c.target_position.x - ex,
                            y = c.target_position.y - ey,
                        },
                    })
                end
            end

            -- Prototype-level view, to cross-check against the data.raw dump.
            for _, proto in ipairs(entity.prototype.fluidbox_prototypes) do
                for _, pc in ipairs(proto.pipe_connections) do
                    local rec = { flow_direction = pc.flow_direction, positions = {} }
                    if pc.positions then
                        for _, p in ipairs(pc.positions) do
                            table.insert(rec.positions, { x = p.x, y = p.y })
                        end
                    end
                    table.insert(entry.prototype_positions, rec)
                end
            end

            table.insert(out.pumpjacks, entry)
        end
    end

    helpers.write_file("oracle-dump.json", helpers.table_to_json(out), false)
end

script.on_init(collect)
