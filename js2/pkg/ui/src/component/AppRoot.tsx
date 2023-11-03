import { ActionIcon, AppShell, Burger, Group, MantineProvider, rem } from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { IconListSearch } from "@tabler/icons-react";
import { PropsWithChildren, ReactNode } from "react";

export function AppRoot(props: PropsWithChildren): ReactNode {
  const [opened, { toggle }] = useDisclosure();
  return <MantineProvider>
    <AppShell
      header={{ height: 60 }}
      // navbar={{ width: 300, breakpoint: 'sm', collapsed: { mobile: !opened } }}
      padding="md"
    >
      <AppShell.Header>
        <Burger opened={opened} onClick={toggle} hiddenFrom="sm" size="sm" />

        <Group>
          <div>Semantic</div>

          <ActionIcon color="gray">
            <IconListSearch style={{ width: rem(20), height: rem(20), color: 'black' }} />
          </ActionIcon>
        </Group>
      </AppShell.Header>


      {/*<AppShell.Navbar p="md">Navbar</AppShell.Navbar>*/}

      <AppShell.Main>
        {props.children}
      </AppShell.Main>
    </AppShell>
  </MantineProvider>;

}
