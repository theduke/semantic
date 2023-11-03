import { ReactNode } from 'react';

import { Card, Group, Text, Badge, Menu, ActionIcon, rem } from '@mantine/core';
import { IconEye, IconFileZip, IconPencil, IconTrash } from '@tabler/icons-react';
import { Link } from 'react-router-dom';

export interface EntityBoxProps {
  id?: string;
	title: ReactNode;
	entityType?: ReactNode;
	children?: ReactNode;
}

export function EntityBox(props: EntityBoxProps): ReactNode {
  const title = props.id ? <Link to={`/entity/${props.id}`}>{props.title}</Link> : props.title;

	return (
		<Card shadow="sm" padding="sm" radius="md" withBorder>
			<Card.Section withBorder inheritPadding mb='xs'>

				<Group justify="space-between" mt="sm" mb="xs">
					<Group>
						<Text fw={700}>{title}</Text>
						<Badge color="pink" variant="light">
							{props.entityType}
						</Badge>
					</Group>

					<Menu withinPortal position="bottom-end" shadow="sm">
						<Menu.Target>
							<ActionIcon size='xl' variant='default'>

								<IconPencil style={{ width: rem(20), height: rem(20), color: 'black' }} />
							</ActionIcon>
						</Menu.Target>

						<Menu.Dropdown>
							<Menu.Item leftSection={<IconFileZip style={{ width: rem(14), height: rem(14) }} />}>
								Download zip
							</Menu.Item>
							<Menu.Item leftSection={<IconEye style={{ width: rem(14), height: rem(14) }} />}>
								Preview all
							</Menu.Item>
							<Menu.Item
								leftSection={<IconTrash style={{ width: rem(14), height: rem(14) }} />}
								color="red"
							>
								Delete all
							</Menu.Item>
						</Menu.Dropdown>
					</Menu>

				</Group>
			</Card.Section>

			<Card.Section inheritPadding mb='xs'>
				{props.children}
			</Card.Section>

		</Card>
	);
}
