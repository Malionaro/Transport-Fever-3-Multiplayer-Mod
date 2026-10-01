#!/usr/bin/env python3
"""Builds the mod's gui/menu/main_page.tl from the game's own file.

The mod's copy is the game's main_page.tl plus the Multiplayer entry
(docs/LOBBY.md): a column of two cards right of the game's own
(Multiplayer, and Join a friend), a top-bar button and the window they
open. Each addition is applied at an anchor that must match exactly once,
so a game patch that moves things fails loudly here rather than in the
game.

    python tools/lobby/make_main_page.py <the game's gui/menu/main_page.tl> [--install <mods folder>]

The game's file is in base/content/gui.zip (`unzip gui.zip gui/menu/main_page.tl`).
With --install, the whole mod is copied into a mods folder (TF3's staging_area).
"""
import argparse
import json
import os
import re
import shutil
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MOD = os.path.join(REPO, "mod", "tpf3mp_1")
OUT = os.path.join(MOD, "content", "gui", "menu", "main_page.tl")

HEADER = '''-- TPF3-MP: the game's gui/menu/main_page.tl with a Multiplayer entry added to
-- the main menu. The hook serves this copy in place of the game's file
-- (docs/LOBBY.md). Every change is marked "TPF3-MP:" so a game patch can be
-- re-applied: tools/lobby/make_main_page.py takes the new game file and
-- re-applies the marked blocks. Relative and leading-slash paths are made
-- absolute ("::/") so the copy resolves the game's files from the mod's folder.
'''

LOBBY_WINDOW = '''-- TPF3-MP: the Multiplayer window, opened from the main menu into the menu's
-- own window container (as DeluxeContentWindow is). Its content is the mod's
-- gui/menu/lobby.lua: the lobby, talking to the hook.
local record LobbyModule
	content : function(onClose : function(), focus : string) : TreeNodeId
	CardLine : function(params : any) : TreeNodeId
	joinLine : function(state : any) : string
end
local lobby = ug_require "tpf3mp_1::/gui/menu/lobby.lua" as LobbyModule

local record Tpf3mpLobbyWindowParam
	onClose : function()
	pos : Vec2f
	focus : string
end

-- TPF3-MP: the lobby's content, or, should its Lua fail, the error and a way
-- out - never a window that cannot be closed.
local function safeContent(onClose : function(), focus : string) : TreeNodeId
	local ok, result = pcall(lobby.content, onClose, focus)
	if ok then
		return result as TreeNodeId
	end
	pcall(debugPrint, "[tpf3mp] lobby: content failed: " .. tostring(result))
	return builtin.BoxLayout{
		orientation = builtin.type.Orientation.Vertical,
		children = {
			builtin.TextView{ meta = { class = "font-scale-title-4" }, text = _("Multiplayer") },
			builtin.TextView{ meta = { class = "font-scale-body, error" }, text = tostring(result) },
			builtin.Button{
				meta = { class = "secondary" },
				content = builtin.TextView{ text = _("Close") },
				onClick = onClose,
			},
		},
	}
end

local Tpf3mpLobbyWindow = react.RegisterWrapperRecipe("Tpf3mpLobbyWindow", builtin.Window, function(param : Tpf3mpLobbyWindowParam) : TreeNodeId
	-- Centred on the screen at any resolution, as the game centres its mod
	-- validation report (mod_manager_react_util.tl): by that window's class,
	-- whose rule in the game's style sheet (mod_browser.css.lua,
	-- "Window!validation-report-dialog") sets anchorPoint and gravity to
	-- 0.5, 0.5 and nothing else. A wrapper recipe may pass meta only for its
	-- class: a styleSheet with anchorPoint made the game assert and close
	-- ("Wrapper recipe must return child", 2026-09-30). initialX and
	-- initialY are shares of the screen for the window's anchor point, so
	-- none is given: the class's gravity places it.
	return builtin.Window{
		title = _("Multiplayer"),
		id = "window.tpf3mp.lobby",
		meta = { class = "fade-in, validation-report-dialog" },
		movable = false,
		closable = true,
		onClose = param.onClose,
		content = safeContent(param.onClose, param.focus),
	}
end)

'''

SHOW = '''	-- TPF3-MP: open the Multiplayer window, as showDeluxeContent opens its
	-- window; `focus` "join" puts joining by invite first.
	local showMultiplayer = function(focus : string)
		local pos = api.type.Vec2f.new(0.5, 0.5)
		titleIconOnlyState:set(true)
		local wc = mainPageParams.commonParams.windowContainer:get():getApi()
		wc.addSingletonWindow(Tpf3mpLobbyWindow, {
			onClose = function()
				titleIconOnlyState:set(false)
				fastFadeInState:set(true)
				cardsFadeInStartTimeRef:set(api.util.getApplicationTime())
				mainPageParams.commonParams.windowContainer:get():getApi().removeAllWindows(Tpf3mpLobbyWindow)
			end,
			pos = pos,
			focus = focus,
		})
	end

	if titleIconOnlyState:old() then
		react.setStyleClasses("title-icon-only")'''

TOPBAR_BUTTON = '''	-- TPF3-MP: the Multiplayer button in the top bar, a glyph drawn as the
	-- game's own top-bar icons are (tools/art/icons/menu_icon.py).
	local multiplayer = button_react_util.makeIconButton(nil, "tpf3mp_1::/gui/tpf3mp/icons/menu_multiplayer_50.tga", function()
		if clickAllowed("TopBar") then
			showMultiplayer(nil)
		end
	end, _("Multiplayer"))

	local settings = button_react_util.makeIconButton(nil, "::/gui/menu/icons/settings_50.tga", function()'''

CARD = '''	-- TPF3-MP: the Multiplayer cards, a column right of the game's own cards:
	-- Multiplayer (connect, create or join) and Join a friend (the window with
	-- the invite first). Each card's line under its title is live, from the
	-- lobby the hook has (lobby.CardLine). The label is the game's own
	-- (menu_icon_react_util.makeCardLabelBottomComponent), with that line in
	-- place of the fixed description.
	local tpf3mpCardLabel = function(title : string, line : function(any) : string) : TreeNodeId
		return builtin.FloatingLayout{
			children = {
				builtin.FloatingLayoutChild{
					item = builtin.ShaderQuad{
						meta = {
							mouseTransparent = true,
						},
						scaling = builtin.type.ImageViewScaling.AutoZoom,
						path0 = "::/gui/menu/design/blackOpaque_effects.tga",
						path1 = nil, -- nrm
						path2 = "::/gui/menu/design/allgreen_masks.tga",
						path3 = "::/gui/menu/design/allwhite_main.tga",
						specularColor = api.type.Vec3f.new(0.42, 0.75, 0.87),
						specularMixAmount = 1.0,
						animatedRippleStrength = 0.18,
						mouseGradDist = 360.0,
						mouseGradBaseStr = 0.34,
						mouseClickStr = 0.51,
						useFullOpacity = false,
						rippleEffectOnClick = true,
						useNormalMap = true,
						mouseGradAdditional = true,
					},
				},
				builtin.FloatingLayoutChild{
					item = builtin.Component{
						layout = builtin.BoxLayout{
							orientation = builtin.type.Orientation.Horizontal,
							children = {
								builtin.Component{
									meta = { class = "title-and-description", },
									layout = builtin.BoxLayout{
										orientation = builtin.type.Orientation.Vertical,
										children = {
											builtin.TextView{
												meta = { class = "font-scale-main-card-title" },
												text = title,
											},
											lobby.CardLine{ line = line },
										},
									},
								},
								gui_react_util.makeHorizontalSpacer(),
							},
						},
					},
				},
			},
		}
	end

	local multiplayerCard = menu_icon_react_util.CardButton{
		bottomComponent = tpf3mpCardLabel(_("Multiplayer"), nil),
		onClick = function()
			if clickAllowed("Cards") then
				showMultiplayer(nil)
			end
		end,
		onAttention = function(x : number, y : number)
			mainPageParams.commonParams.triggerBackgroundEvent(api.type.Vec2f.new(x, y))
		end,
		tooltip = _("Play together online: connect, create a room or join one"),
		images = { "::/gui/menu/images/m02_ingame.tga", "::/gui/menu/images/m07_ingame.tga" },
		initialImageIndex = 1,
		imageSwapOffsetSeconds = 0.41 * 36.0,
		displayDurationSeconds = 36.0,
		class = "small-card, top-right",
		extraChildren = {
			builtin.FloatingLayoutChild{
				h = 0.06,
				v = 0.08,
				item = builtin.ImageView{
					meta = { mouseTransparent = true },
					path = "tpf3mp_1::/gui/tpf3mp/icons/menu_multiplayer_50.tga",
				},
			},
		},
	}

	local joinCard = menu_icon_react_util.CardButton{
		bottomComponent = tpf3mpCardLabel(_("Join a friend"), lobby.joinLine),
		onClick = function()
			if clickAllowed("Cards") then
				showMultiplayer("join")
			end
		end,
		onAttention = function(x : number, y : number)
			mainPageParams.commonParams.triggerBackgroundEvent(api.type.Vec2f.new(x, y))
		end,
		tooltip = _("Join a friend's room with the invite code they send you"),
		images = { "::/gui/menu/images/m05_ingame.tga" },
		initialImageIndex = 1,
		class = "small-card",
	}

	local multiplayerColumn = vBox({
		vBox({ multiplayerCard }, "level2b"),
		vBox({ joinCard }, "level2b"),
	}, "level1")

	local saveId = api.type.SavegameId.new()'''


def build(source_text):
    """The mod's main_page.tl from the game's."""
    text = source_text

    def edit(anchor, new, count=1):
        nonlocal text
        n = text.count(anchor)
        if n != count:
            raise SystemExit(f"anchor found {n}x, expected {count}: {anchor[:70]!r}")
        text = text.replace(anchor, new)

    edit('local react = ug_require "/gui/main/react.lua" as React\n',
         HEADER + 'local react = ug_require "/gui/main/react.lua" as React\n')
    edit('local table_util = ug_require "/scripts/table_util.tl" as TableUtil\n',
         'local table_util = ug_require "/scripts/table_util.tl" as TableUtil\n'
         '\n-- TPF3-MP: the game log shows the copy is in effect, even if nothing is clicked.\n'
         'pcall(debugPrint, "[tpf3mp] main menu: TPF3-MP main_page.tl is in effect")\n')
    edit('ug_require "configs/deluxe_content.lua"', 'ug_require "::/gui/menu/configs/deluxe_content.lua"')
    edit('ug_require "releasenotes_window.tl"', 'ug_require "::/gui/menu/releasenotes_window.tl"')
    edit('local GameEditionTextMainPage = react.RegisterRecipe(',
         LOBBY_WINDOW + 'local GameEditionTextMainPage = react.RegisterRecipe(')
    edit('	if titleIconOnlyState:old() then\n		react.setStyleClasses("title-icon-only")', SHOW)
    edit('	local settings = button_react_util.makeIconButton(nil, "::/gui/menu/icons/settings_50.tga", function()',
         TOPBAR_BUTTON)
    edit('					gui_react_util.makeHorizontalSpacer(),\n					settings, ',
         '					gui_react_util.makeHorizontalSpacer(),\n					multiplayer, -- TPF3-MP\n					settings, ')
    edit('	local saveId = api.type.SavegameId.new()', CARD)
    # The Multiplayer column has the grid's top-right corner now.
    edit('		mapEditorDisplayDuration,\n		"small-card, top-right"\n',
         '		mapEditorDisplayDuration,\n		"small-card" -- TPF3-MP: was "small-card, top-right"; the Multiplayer card has that corner\n')
    edit('	local cardWrap = vBox({\n		hBox({\n			newGameCard, ',
         '	local cardWrap = vBox({\n		hBox({ vBox({ -- TPF3-MP: the game\'s cards, then the Multiplayer column\n		hBox({\n			newGameCard, ')
    edit('			campaignCard\n		}, "level1"),\n	}, "card-wrap", cardWrapRef)',
         '			campaignCard\n		}, "level1"),\n		}), multiplayerColumn }), -- TPF3-MP\n	}, "card-wrap", cardWrapRef)')

    # A leading-slash path is resolved against the requiring file's root, which
    # for the mod's copy is tpf3mp_1::/; name the game's root explicitly.
    text = re.sub(r'"/(gui|scripts|base|mission)/', lambda m: '"::/' + m.group(1) + '/', text)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", help="the game's gui/menu/main_page.tl")
    parser.add_argument("--install", metavar="MODS", help="also copy the mod into this mods folder")
    args = parser.parse_args()

    source = open(args.source, encoding="utf-8").read()
    text = build(source)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as out:
        out.write(text)
    print(f"wrote {OUT} ({len(text.splitlines())} lines)")

    content = os.path.join(MOD, "_content.json")
    listing = json.load(open(content, encoding="utf-8"))
    for path in ("gui/menu/main_page.tl", "gui/menu/lobby.lua", "tpf3mp/state.lua", "tpf3mp/act.lua",
                 "gui/tpf3mp/icons/menu_multiplayer_50.tga", "gui/tpf3mp/icons/menu_multiplayer_50@2x.tga"):
        if path not in listing["files"]:
            listing["files"].insert(0, path)
            print(f"listed {path} in _content.json")
    with open(content, "w", encoding="utf-8", newline="\n") as out:
        out.write(json.dumps(listing, indent=4) + "\n")

    if args.install:
        dest = os.path.join(args.install, "tpf3mp_1")
        if os.path.isdir(dest):
            shutil.rmtree(dest)
        shutil.copytree(MOD, dest)
        print(f"installed -> {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
