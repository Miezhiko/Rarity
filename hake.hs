{-# LANGUAGE MultiWayIf    #-}
{-# LANGUAGE UnicodeSyntax #-}

import           Hake

main ∷ IO ()
main = hake $ do

  "clean | clean the project" ∫
    cargo ["clean"] >> removeDirIfExists targetPath

  "update | update dependencies" ∫ cargo ["update"]

  rarityExecutable ♯
      git ["submodule", "update", "--init"]
   >> cargo <| "build" : buildFlags

  "install | install to system" ◉ [rarityExecutable] ∰
    cargo <| "install" : buildFlags

  "test | build and test" ◉ [rarityExecutable] ∰ do
    cargo ["test"]
    cargo ["clippy"]
    rawSystem rarityExecutable ["--version"]
      >>= checkExitCode

  "run | run rarity" ◉ [rarityExecutable] ∰
    cargo . (("run" : buildFlags) ++) . ("--" :) =<< getHakeArgs

 where
  targetPath ∷ FilePath
  targetPath = "target"

  buildPath ∷ FilePath
  buildPath = targetPath </> "release"

  buildFlags ∷ [String]
  buildFlags = [ "--release" ]

  rarityExecutable ∷ FilePath
  rarityExecutable =
    {- HLINT ignore "Redundant multi-way if" -}
    if | os ∈ ["win32", "mingw32", "cygwin32"] -> buildPath </> "rarity.exe"
       | otherwise                             -> buildPath </> "rarity"
