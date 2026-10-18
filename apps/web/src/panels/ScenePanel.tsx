// Right panel: the palette, then the properties in three tabs (element, scene, run).
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs.tsx'
import { useEditor, type PanelTab } from '../editor/store.ts'
import { ElementPanel } from './ElementPanel.tsx'
import { Palette } from './Palette.tsx'
import { RunPanel } from './RunPanel.tsx'
import { ScenePropertiesPanel } from './ScenePropertiesPanel.tsx'

export function ScenePanel() {
  const tab = useEditor((s) => s.panelTab)
  const setTab = useEditor((s) => s.setPanelTab)
  return (
    <aside className="flex w-80 shrink-0 flex-col gap-5 overflow-y-auto border-l border-border p-3 text-sm">
      <Palette />
      <Tabs value={tab} onValueChange={(v) => setTab(v as PanelTab)}>
        <TabsList>
          <TabsTrigger value="element" data-testid="tab-element">
            Element
          </TabsTrigger>
          <TabsTrigger value="scene" data-testid="tab-scene">
            Scene
          </TabsTrigger>
          <TabsTrigger value="run" data-testid="tab-run">
            Run
          </TabsTrigger>
        </TabsList>
        <TabsContent value="element">
          <ElementPanel />
        </TabsContent>
        <TabsContent value="scene">
          <ScenePropertiesPanel />
        </TabsContent>
        <TabsContent value="run">
          <RunPanel />
        </TabsContent>
      </Tabs>
    </aside>
  )
}
