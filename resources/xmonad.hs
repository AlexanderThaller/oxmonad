import XMonad
import XMonad.Actions.CycleWS         -- For Workspace Cycling
import XMonad.Actions.PhysicalScreens -- For better ScreenManagement
import XMonad.Hooks.DynamicLog        -- For xmobar
import XMonad.Hooks.EwmhDesktops      -- For _NET_ACTIVE_WINDOW so we can grab the current window
import XMonad.Hooks.SetWMName         -- For fixing java applications with xmonad
import XMonad.Layout.Grid             -- For Grid Layout
import XMonad.Layout.NoBorders        -- For noBorder Layouts
import XMonad.Layout.Reflect
import XMonad.Layout.ResizableTile    -- For resizableTile
import XMonad.Layout.Tabbed
import XMonad.Layout.ThreeColumns     -- For a three column layout. good for wide screens
import XMonad.Layout.LayoutScreens
import XMonad.Layout.TwoPane
import XMonad.Util.CustomKeys         -- For myKeys
import XMonad.Util.EZConfig           -- For Additional media key bindings

-- For DynamicWorkspaces
import XMonad.Actions.DynamicWorkspaces
import XMonad.Prompt
import XMonad.Actions.CopyWindow(copy)
import qualified XMonad.StackSet as W

-- Main
main = xmonad =<< statusBar myBar myPP toggleStrutsKey myConfig

-- Config
myTerminal           = "alacritty"
--myModMask            = mod4Mask
myModMask            = mod1Mask
-- myTerminal           = "terminator"
myBorderWidth        = 1
myNormalBorderColor  = colorLightGrey
myFocusedBorderColor = colorRed

-- Colors
colorBlack      =  "#000000"
colorBlue       =  "#1565C1"
colorGrey       =  "#1C1C1C"
colorLightGrey  =  "#969694"
colorRed        =  "#C62828"
colorWhite      =  "#F2F2F2"
colorYellow     =  "#F8E63A"
colorGreen      =  "#86BD47"
colorDarkGrey   =  "#161616"

-- Xmobar Config
myBar = "xmobar --screen=0"
myPP  = xmobarPP
  { ppCurrent = xmobarColor colorYellow "" . wrap "<" ">"
  , ppTitle = xmobarColor colorGreen "" . shorten 100
  }
toggleStrutsKey XConfig {XMonad.modMask = modMask} = (modMask, xK_b)

-- Prompt Config
myXPConfig :: XPConfig
myXPConfig = def { bgColor     = colorDarkGrey
                 , fgColor     = colorWhite
                 , borderColor = colorWhite
                 , fgHLight    = colorWhite
                 , bgHLight    = colorBlue
                 , font        = "xft:Fira Mono:size=16:antialias=true"
                 }

-- KeyBindings
myKeys = customKeys delkeys addkeys
  where
    -- Keys to delete
    delkeys :: XConfig l -> [(KeyMask, KeySym)]
    delkeys XConfig {modMask = modm} =
      -- Delete Quit command for xmonad
      [ --(modm .|. shiftMask, xK_q )
      ]

   -- Keys to add
    addkeys :: XConfig l -> [((KeyMask, KeySym), X ())]
    addkeys conf@(XConfig {modMask = modm}) =
      -- Dmenu
      [ ((modm              , xK_p        ), spawn "dmenu_run -fn 'Fira Mono-14' -nb '#161616' -nf '#F2F2F2' -sb '#F9A825'")
      -- Swap Workspace between screens
      , ((modm              , xK_z        ), swapNextScreen)
      -- Swap Workspace with previous workspace
      , ((modm .|. shiftMask, xK_z        ), toggleWS' ["NSP"])
      -- lock screen
      , ((modm .|. shiftMask, xK_l        ), spawn "slock")
      -- launch editor
      , ((modm, xK_x                      ), spawn "emacs")
      -- resize Tile, expand
      , ((modm              , xK_Up       ), sendMessage MirrorExpand)
      -- resize Tile, shrink
      , ((modm              , xK_Down     ), sendMessage MirrorShrink)
      , ((modm              , xK_Down     ), sendMessage MirrorShrink)
      -- dynamic workspaces
      , ((modm .|. shiftMask, xK_BackSpace), removeWorkspace) -- remove
      , ((modm              , xK_v        ), selectWorkspace myXPConfig) -- select
      , ((modm              , xK_m        ), withWorkspace myXPConfig (windows . W.shift)) -- move
      , ((modm .|. shiftMask, xK_m        ), withWorkspace myXPConfig (windows . copy)) -- copy
      , ((modm .|. shiftMask, xK_r        ), renameWorkspace myXPConfig)  -- rename
      , ((modm, xK_Right                  ), onNextNeighbour def W.view)
      , ((modm, xK_Left                   ), onPrevNeighbour def W.view)
      , ((modm .|. shiftMask, xK_Right    ), onNextNeighbour def W.shift)
      , ((modm .|. shiftMask, xK_Left     ), onPrevNeighbour def W.shift)
      , ((0                   , 0x1008ff18), spawn "playerctl play-pause")
      , ((0                   , 0x1008FF12), spawn "playerctl play-pause")
      -- XF86AudioPlay
      , ((0                   , 0x1008ff14), spawn "playerctl play-pause")
      -- XF86AudioNext
      , ((0                   , 0x1008ff17), spawn "playerctl next")
      -- XF86AudioPrev
      , ((0                   , 0x1008ff16), spawn "playerctl previous")
      -- ScreenDefault
      , ((modm .|. shiftMask, xK_s), layoutSplitScreen 2 (TwoPane 0.5 0.5))
      , ((modm .|. controlMask .|. shiftMask, xK_s), XMonad.Actions.PhysicalScreens.rescreen def)
      ]

-- Workspaces
myWorkspaces = map show [1..9]

-- Layouts
myLayout = (reflectHoriz $ tiled) ||| tiled ||| Mirror tiled ||| noBorders Full ||| tabbed shrinkText myTabConf ||| Mirror Grid ||| Grid ||| threeColsTwo ||| (reflectHoriz $ threeColsTwo)
  where
    -- default tiling algorithm partitions the screen into two panes
    tiled   = ResizableTall nmaster delta ratio []

    -- The default number of windows in the master pane
    nmaster = 1

    -- Default proportion of screen occupied by master pane
    ratio   = 3/5

    -- Percent of screen to increment by when resizing panes
    delta   = 3/100

    threeCols = ThreeColMid 1 (3/100) (1/2)
    threeColsTwo = ThreeCol 1 (3/100) (1/2)

myTabConf :: Theme
myTabConf = def {
  fontName = "xft:Fira Mono:size=12:antialias=true"
}

-- https://www.peterstuart.org/posts/2021-09-06-xmonad-zoom/
manageZoomHook =
  composeAll $
    [ (className =? zoomClassName) <&&> shouldFloat <$> title --> doFloat,
      (className =? zoomClassName) <&&> shouldSink <$> title --> doSink
    ]
  where
    zoomClassName = "zoom"
    tileTitles =
      [ "Zoom - Free Account", -- main window
        "Zoom - Licensed Account", -- main window
        "Zoom", -- meeting window on creation
        "Zoom Meeting" -- meeting window shortly after creation
      ]
    shouldFloat title = title `notElem` tileTitles
    shouldSink title = title `elem` tileTitles
    doSink = (ask >>= doF . W.sink) <+> doF W.swapDown

-- ManageHooks
myManageHook = manageZoomHook

-- MyConfig
myConfig = def
  { terminal           = myTerminal
  , borderWidth        = myBorderWidth
  , modMask            = myModMask
  , keys               = myKeys
  , layoutHook         = myLayout
  , normalBorderColor  = myNormalBorderColor
  , focusedBorderColor = myFocusedBorderColor
  , workspaces         = myWorkspaces
  , manageHook         = myManageHook
  -- Make java work with xmonad see:
  -- http://thinkingeek.com/2012/01/24/fix-java-gui-applications-xmonad/
  , startupHook        = setWMName "LG3D"
  }
